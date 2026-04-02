#![allow(clippy::await_holding_refcell_ref)] // clippy is dumb

use core::{
    cell::{Ref, RefCell},
    cmp,
    ops::Deref,
    slice,
};

use std::{io, net::Shutdown};

pub use rustls_crate::*;

use rustls_crate::{
    client::UnbufferedClientConnection,
    server::UnbufferedServerConnection,
    unbuffered::UnbufferedConnectionCommon,
    unbuffered::{ConnectionState, EncryptError, UnbufferedStatus},
};

use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncBufRead, AsyncBufWrite, BoundedBuf, BoundedBufMut},
};

/// Trait to abstract over `UnbufferedServerConnection` and `UnbufferedClientConnection`,
/// since `process_tls_records` is not on a shared trait in rustls.
#[doc(hidden)]
pub trait ProcessTlsRecords: sealed::Sealed {
    type Data;

    fn process_tls_records<'c, 'i>(&'c mut self, incoming_tls: &'i mut [u8]) -> UnbufferedStatus<'c, 'i, Self::Data>;
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::UnbufferedServerConnection {}
    impl Sealed for super::UnbufferedClientConnection {}
}

impl ProcessTlsRecords for UnbufferedServerConnection {
    type Data = server::ServerConnectionData;

    fn process_tls_records<'c, 'i>(&'c mut self, incoming_tls: &'i mut [u8]) -> UnbufferedStatus<'c, 'i, Self::Data> {
        let inner: &mut UnbufferedConnectionCommon<Self::Data> = self;
        inner.process_tls_records(incoming_tls)
    }
}

impl ProcessTlsRecords for UnbufferedClientConnection {
    type Data = client::ClientConnectionData;

    fn process_tls_records<'c, 'i>(&'c mut self, incoming_tls: &'i mut [u8]) -> UnbufferedStatus<'c, 'i, Self::Data> {
        let inner: &mut UnbufferedConnectionCommon<Self::Data> = self;
        inner.process_tls_records(incoming_tls)
    }
}

/// Reduced `ConnectionState` that doesn't borrow the connection or incoming buffer.
/// Created by draining all needed data from the borrowed state variants.
/// A TLS stream type that supports concurrent async read/write through [AsyncBufRead] and
/// [AsyncBufWrite] traits.
///
/// [AsyncBufRead::read] and [AsyncBufWrite::write] can be polled concurrently from separate
/// tasks. The read path owns `read_buf` during IO and the write path owns `write_buf`, so
/// neither blocks the other while awaiting kernel completions.
///
/// # Panics
/// Each async read/write operation must be polled to completion. Dropping a future before it
/// completes will leave internal buffers in a taken state, causing the next call to panic.
pub struct TlsStream<C, Io> {
    io: Io,
    session: RefCell<Session<C>>,
}

struct Session<C> {
    conn: C,
    read_buf: Option<BytesMut>,
    /// Write buffer for application data (used by write path).
    write_buf: Option<BytesMut>,
    /// Write buffer for TLS protocol responses during reads (key updates, alerts).
    proto_write_buf: BytesMut,
    /// Plaintext buffered from a previous read.
    pending_plaintext: BytesMut,
}

impl<C, Io> TlsStream<C, Io>
where
    C: Deref<Target: Deref<Target = CommonState>>,
{
    /// Returns the negotiated ALPN protocol, if any.
    pub fn session(&self) -> Ref<'_, CommonState> {
        let session = self.session.borrow();
        Ref::map(session, |session| &**session.conn)
    }
}

impl<C, Io> TlsStream<C, Io>
where
    C: ProcessTlsRecords,
    Io: AsyncBufRead + AsyncBufWrite,
{
    pub async fn handshake(io: Io, conn: C) -> io::Result<Self> {
        let stream = TlsStream {
            io,
            session: RefCell::new(Session {
                conn,
                read_buf: Some(BytesMut::new()),
                write_buf: Some(BytesMut::new()),
                proto_write_buf: BytesMut::new(),
                pending_plaintext: BytesMut::new(),
            }),
        };
        stream._handshake().await?;
        Ok(stream)
    }

    async fn _handshake(&self) -> io::Result<()> {
        let mut session = self.session.borrow_mut();
        let mut read_buf = session.read_buf.take().expect(POLL_TO_COMPLETE);
        let mut proto_write_buf = session.proto_write_buf.split();

        let res = loop {
            let UnbufferedStatus { discard, state } = session.conn.process_tls_records(read_buf.as_mut());

            let res = match state.map_err(tls_err) {
                Err(e) => {
                    read_buf.advance(discard);
                    Err(e)
                }

                Ok(ConnectionState::EncodeTlsData(mut state)) => {
                    let enc_res = encode_tls_data(&mut state, &mut proto_write_buf);
                    drop(state);
                    read_buf.advance(discard);
                    enc_res?;
                    continue;
                }

                Ok(ConnectionState::TransmitTlsData(state)) => {
                    state.done();
                    read_buf.advance(discard);

                    let (res, b) = write_all_buf(&self.io, proto_write_buf).await;
                    proto_write_buf = b;
                    res
                }

                Ok(ConnectionState::BlockedHandshake) => {
                    read_buf.advance(discard);

                    let (res, b) = read_to_buf(&self.io, read_buf).await;
                    read_buf = b;
                    res
                }

                Ok(ConnectionState::WriteTraffic(_) | ConnectionState::ReadTraffic(_)) => {
                    read_buf.advance(discard);
                    break Ok(());
                }

                Ok(ConnectionState::PeerClosed | ConnectionState::Closed) => {
                    read_buf.advance(discard);
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, "tls handshake eof"))
                }

                Ok(_) => {
                    read_buf.advance(discard);
                    continue;
                }
            };

            if res.is_err() {
                break res;
            }
        };

        session.read_buf.replace(read_buf);
        session.proto_write_buf = proto_write_buf;
        res
    }
}

impl<C, Io> TlsStream<C, Io>
where
    C: ProcessTlsRecords,
    Io: AsyncBufRead,
{
    /// Read ciphertext from IO, decrypt, and return plaintext.
    async fn read_tls(&self, plain_buf: &mut impl BoundedBufMut) -> io::Result<usize> {
        let mut session = self.session.borrow_mut();

        // Check for plaintext buffered from a previous read first.
        if !session.pending_plaintext.is_empty() {
            let rem = plain_buf.bytes_total() - plain_buf.bytes_init();
            let aval = session.pending_plaintext.len();
            let len = cmp::min(rem, aval);

            plain_buf.put_slice(&session.pending_plaintext[..len]);
            session.pending_plaintext.advance(len);

            return Ok(len);
        }

        let mut read_buf = session.read_buf.take().expect(POLL_TO_COMPLETE);

        let res = loop {
            // Call process_tls_records directly to copy record payload
            // straight into the caller's buffer (no intermediate BytesMut).
            let session_ref = &mut *session;

            let UnbufferedStatus { discard, state } = session_ref.conn.process_tls_records(read_buf.as_mut());

            let res = match state.map_err(tls_err) {
                Err(e) => {
                    read_buf.advance(discard);
                    break Err(e);
                }

                Ok(ConnectionState::ReadTraffic(mut traffic)) => {
                    let rem = plain_buf.bytes_total() - plain_buf.bytes_init();
                    let mut written = 0;

                    let mut err = None;
                    while let Some(res) = traffic.next_record() {
                        match res.map_err(tls_err) {
                            Ok(record) => {
                                let payload = record.payload;
                                let len = payload.len().min(rem - written);

                                let (head, tail) = payload.split_at(len);

                                plain_buf.put_slice(head);
                                written += len;

                                // Buffer overflow into pending_plaintext.
                                session_ref.pending_plaintext.extend_from_slice(tail);
                            }
                            Err(e) => {
                                err = Some(e);
                                break;
                            }
                        }
                    }

                    read_buf.advance(discard);

                    if let Some(e) = err {
                        break Err(e);
                    }

                    // Empty plaintext means TLS overhead with no payload — keep going.
                    if written == 0 {
                        continue;
                    }

                    break Ok(written);
                }

                Ok(ConnectionState::EncodeTlsData(mut state)) => {
                    // Encode into proto_write_buf via session_ref (same borrow scope as state).
                    let enc_res = encode_tls_data(&mut state, &mut session_ref.proto_write_buf);
                    drop(state);
                    read_buf.advance(discard);

                    if let Err(e) = enc_res {
                        break Err(e);
                    }
                    continue;
                }

                Ok(ConnectionState::TransmitTlsData(state)) => {
                    // Data is in proto_write_buf. Acknowledge and continue —
                    // write_tls will flush it on the next write call.
                    state.done();
                    read_buf.advance(discard);
                    continue;
                }

                // Need more ciphertext.
                Ok(ConnectionState::BlockedHandshake | ConnectionState::WriteTraffic(_)) => {
                    read_buf.advance(discard);

                    drop(session);

                    let (res, b) = read_to_buf(&self.io, read_buf).await;
                    read_buf = b;

                    session = self.session.borrow_mut();

                    res
                }

                Ok(ConnectionState::PeerClosed | ConnectionState::Closed) => {
                    read_buf.advance(discard);
                    break Ok(0);
                }

                Ok(_) => {
                    read_buf.advance(discard);
                    continue;
                }
            };

            if let Err(e) = res {
                break Err(e);
            }
        };

        session.read_buf.replace(read_buf);
        res
    }
}

impl<C, Io> TlsStream<C, Io>
where
    C: ProcessTlsRecords,
    Io: AsyncBufWrite,
{
    /// Encrypt plaintext and write all ciphertext to IO.
    async fn write_tls(&self, plain: &impl BoundedBuf) -> io::Result<usize> {
        let mut session = self.session.borrow_mut();
        let mut write_buf = session.write_buf.take().expect(POLL_TO_COMPLETE);
        let plaintext = plain.chunk();

        // Flush protocol data buffered by read path (key updates, alerts).
        if !session.proto_write_buf.is_empty() {
            write_buf.extend_from_slice(&session.proto_write_buf);
            session.proto_write_buf.clear();
        }

        let res = loop {
            // Pass empty slice — write path doesn't process incoming TLS records.
            // Incoming data (key updates, etc.) is handled by the read path.
            let UnbufferedStatus { state, .. } = session.conn.process_tls_records(&mut []);

            match state.map_err(tls_err) {
                Err(e) => break Err(e),

                Ok(ConnectionState::WriteTraffic(mut traffic)) => {
                    if let Err(e) = encrypt_to_buf(&mut traffic, plaintext, &mut write_buf) {
                        break Err(e);
                    }

                    drop(session);

                    let (res, b) = write_all_buf(&self.io, write_buf).await;
                    write_buf = b;

                    session = self.session.borrow_mut();

                    break res.map(|_| plaintext.len());
                }

                Ok(ConnectionState::EncodeTlsData(mut state)) => {
                    if let Err(e) = encode_tls_data(&mut state, &mut write_buf) {
                        break Err(e);
                    }
                }

                Ok(ConnectionState::TransmitTlsData(state)) => {
                    state.done();

                    drop(session);

                    let (res, b) = write_all_buf(&self.io, write_buf).await;
                    write_buf = b;

                    session = self.session.borrow_mut();

                    if let Err(e) = res {
                        break Err(e);
                    }
                }

                Ok(ConnectionState::PeerClosed | ConnectionState::Closed) => {
                    break Err(io::ErrorKind::UnexpectedEof.into());
                }

                Ok(_) => {}
            }
        };

        session.write_buf.replace(write_buf);
        res
    }

    /// Send a TLS close_notify alert, flushing any pending data first.
    async fn tls_shutdown(&self) -> io::Result<()> {
        // Flush pending application/protocol data.
        self.write_tls(&Vec::new()).await?;

        let mut session = self.session.borrow_mut();
        let mut write_buf = session.write_buf.take().expect(POLL_TO_COMPLETE);

        loop {
            let UnbufferedStatus { state, .. } = session.conn.process_tls_records(&mut []);

            match state.map_err(tls_err)? {
                ConnectionState::WriteTraffic(mut traffic) => {
                    write_buf.reserve(64);
                    // SAFETY: queue_close_notify writes a single TLS alert record
                    // contiguously from index 0. On Ok(n), exactly n bytes are written.
                    // On InsufficientSize, no bytes are written.
                    let res = unsafe {
                        SpareCapBuf::new(&mut write_buf).with_mut_slice(|spare| traffic.queue_close_notify(spare))
                    };

                    if let Err(EncryptError::InsufficientSize(s)) = res {
                        write_buf.reserve(s.required_size);
                        continue;
                    }

                    drop(session);

                    return write_all_buf(&self.io, write_buf).await.0;
                }
                ConnectionState::EncodeTlsData(mut state) => encode_tls_data(&mut state, &mut write_buf)?,
                ConnectionState::TransmitTlsData(state) => {
                    state.done();
                    drop(session);

                    let (res, b) = write_all_buf(&self.io, write_buf).await;
                    write_buf = b;

                    res?;
                    session = self.session.borrow_mut();
                }
                ConnectionState::PeerClosed | ConnectionState::Closed => return Ok(()),
                _ => {}
            }
        }
    }
}

impl<C, Io> AsyncBufRead for TlsStream<C, Io>
where
    C: ProcessTlsRecords,
    Io: AsyncBufRead,
{
    async fn read<B>(&self, mut buf: B) -> (io::Result<usize>, B)
    where
        B: BoundedBufMut,
    {
        let res = self.read_tls(&mut buf).await;
        (res, buf)
    }
}

impl<C, Io> AsyncBufWrite for TlsStream<C, Io>
where
    C: ProcessTlsRecords,
    Io: AsyncBufWrite,
{
    async fn write<B>(&self, buf: B) -> (io::Result<usize>, B)
    where
        B: BoundedBuf,
    {
        let res = self.write_tls(&buf).await;
        (res, buf)
    }

    async fn shutdown(self, direction: Shutdown) -> io::Result<()> {
        let res = self.tls_shutdown().await;
        let shutdown_res = self.io.shutdown(direction).await;

        res?;
        shutdown_res
    }
}

fn tls_err(e: Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// Read from IO into a BytesMut, reserving space if needed.
async fn read_to_buf(io: &impl AsyncBufRead, mut buf: BytesMut) -> (io::Result<()>, BytesMut) {
    let len = buf.len();
    buf.reserve(4096);

    let (res, b) = io.read(buf.slice(len..)).await;
    buf = b.into_inner();

    match res {
        Ok(0) => (Err(io::ErrorKind::UnexpectedEof.into()), buf),
        Ok(_) => (Ok(()), buf),
        Err(e) => (Err(e), buf),
    }
}

/// Write all bytes from a BytesMut to IO, then clear it.
async fn write_all_buf(io: &impl AsyncBufWrite, mut buf: BytesMut) -> (io::Result<()>, BytesMut) {
    let (res, b) = io.write_all(buf).await;
    buf = b;
    if res.is_ok() {
        buf.clear();
    }
    (res, buf)
}

/// Encode TLS handshake data into the write buffer, resizing if needed.
fn encode_tls_data<Data>(state: &mut unbuffered::EncodeTlsData<'_, Data>, write_buf: &mut BytesMut) -> io::Result<()> {
    // SAFETY: EncodeTlsData::encode copies a single chunk contiguously from index 0.
    // On Ok(n), exactly n bytes are written. On InsufficientSize or AlreadyEncoded,
    // the size check happens before any write so the slice is untouched.
    while let Err(e) = unsafe { SpareCapBuf::new(write_buf).with_mut_slice(|slice| state.encode(slice)) } {
        match e {
            unbuffered::EncodeError::InsufficientSize(unbuffered::InsufficientSizeError { required_size }) => {
                write_buf.reserve(required_size);
            }
            e => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
        }
    }
    Ok(())
}

/// Encrypt plaintext into the write buffer, resizing if needed.
fn encrypt_to_buf<Data>(
    traffic: &mut unbuffered::WriteTraffic<'_, Data>,
    plaintext: &[u8],
    write_buf: &mut BytesMut,
) -> io::Result<()> {
    write_buf.reserve(plaintext.len() + 64);
    // SAFETY: WriteTraffic::encrypt writes TLS records contiguously from index 0 via
    // write_fragments. On Ok(n), exactly n bytes are written. On InsufficientSize,
    // check_required_size returns before any write. On EncryptExhausted, the error
    // is returned during pre-encryption checks before any write.
    while let Err(err) =
        unsafe { SpareCapBuf::new(write_buf).with_mut_slice(|spare| traffic.encrypt(plaintext, spare)) }
    {
        match err {
            EncryptError::InsufficientSize(unbuffered::InsufficientSizeError { required_size }) => {
                write_buf.reserve(required_size);
            }
            e => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
        }
    }
    Ok(())
}

/// Wraps a `BytesMut`'s spare capacity as a mutable byte slice.
///
/// Encapsulates the unsafe operations of interpreting spare capacity as `&mut [u8]`
/// and committing written bytes via `set_len`.
struct SpareCapBuf<'a> {
    buf: &'a mut BytesMut,
}

impl<'a> SpareCapBuf<'a> {
    fn new(buf: &'a mut BytesMut) -> Self {
        Self { buf }
    }

    /// # Safety
    ///
    /// The callback `func` must uphold the following contract:
    /// - Writes must be sequential and contiguous, starting from index 0 of the slice.
    /// - On `Ok(n)`, exactly `n` bytes must have been written to `slice[..n]`.
    /// - On `Err`, zero bytes must have been written into the slice.
    unsafe fn with_mut_slice<F, E>(self, func: F) -> Result<(), E>
    where
        F: FnOnce(&mut [u8]) -> Result<usize, E>,
    {
        let spare = self.buf.spare_capacity_mut();

        // SAFETY: the caller must write into the slice before reading.
        // We only expose this for write-before-read patterns (TLS encode/encrypt).
        let slice = unsafe { slice::from_raw_parts_mut(spare.as_mut_ptr().cast::<u8>(), spare.len()) };

        let n = func(slice)?;

        // SAFETY: caller guarantees n bytes were written into the spare capacity.
        unsafe { self.buf.set_len(self.buf.len() + n) };

        Ok(())
    }
}

const POLL_TO_COMPLETE: &str = "previous call to future didn't polled to completion";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spare_cap_buf_write_and_commit() {
        let mut buf = BytesMut::with_capacity(64);
        buf.extend_from_slice(b"hello");

        let res = unsafe {
            SpareCapBuf::new(&mut buf).with_mut_slice(|slice| {
                assert!(slice.len() >= 59);
                slice[..5].copy_from_slice(b"world");
                Ok::<_, ()>(5)
            })
        };
        assert!(res.is_ok());
        assert_eq!(&buf[..], b"helloworld");
    }

    #[test]
    fn spare_cap_buf_commit_zero() {
        let mut buf = BytesMut::with_capacity(16);
        buf.extend_from_slice(b"abc");

        let res = unsafe { SpareCapBuf::new(&mut buf).with_mut_slice(|_| Ok::<_, ()>(0)) };
        assert!(res.is_ok());
        assert_eq!(&buf[..], b"abc");
    }

    #[test]
    fn spare_cap_buf_error_no_commit() {
        let mut buf = BytesMut::with_capacity(16);
        buf.extend_from_slice(b"abc");

        let res = unsafe { SpareCapBuf::new(&mut buf).with_mut_slice(|_| Err::<usize, _>("too small")) };
        assert!(res.is_err());
        assert_eq!(&buf[..], b"abc");
    }
}
