#![allow(clippy::await_holding_refcell_ref)] // clippy is dumb

use core::{cell::RefCell, slice};

use std::{io, net::Shutdown, rc::Rc};

pub use rustls_crate::*;

use rustls_crate::{
    client::UnbufferedClientConnection,
    server::UnbufferedServerConnection,
    unbuffered::UnbufferedConnectionCommon,
    unbuffered::{ConnectionState, EncryptError, UnbufferedStatus},
};

use xitca_io::{
    bytes::{Buf, BytesMut},
    io_uring::{AsyncBufRead, AsyncBufWrite, BoundedBuf, BoundedBufMut},
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
enum State {
    /// Read traffic was processed (plaintext drained by caller).
    ReadTraffic,
    /// Handshake data was encoded into write_buf.
    EncodedTlsData,
    /// Encoded data needs to be transmitted, then call done.
    TransmitTlsData,
    /// Need more ciphertext.
    BlockedHandshake,
    /// Handshake complete, ready to send app data.
    WriteTraffic,
    /// Peer sent close_notify or connection fully closed.
    Closed,
    /// Peer closed (edge-triggered).
    PeerClosed,
}

/// Process one round of TLS records and return an owned State plus the discard count.
/// This function handles `EncodeTlsData` inline (encoding into write_buf) and
/// `ReadTraffic` inline (draining plaintext into a BytesMut).
fn process_once<C: ProcessTlsRecords>(
    conn: &mut C,
    read_buf: &mut BytesMut,
    write_buf: &mut BytesMut,
) -> io::Result<State> {
    let UnbufferedStatus { discard, state } = conn.process_tls_records(read_buf.as_mut());

    let state = match state.map_err(tls_err)? {
        ConnectionState::ReadTraffic(_) => State::ReadTraffic,
        ConnectionState::EncodeTlsData(mut state) => {
            encode_tls_data(&mut state, write_buf)?;
            State::EncodedTlsData
        }
        ConnectionState::TransmitTlsData(state) => {
            state.done();
            State::TransmitTlsData
        }
        ConnectionState::WriteTraffic(_) => {
            // WriteTraffic may have pending TLS data (key updates).
            // We don't encrypt here — caller decides.
            State::WriteTraffic
        }
        ConnectionState::BlockedHandshake => State::BlockedHandshake,
        ConnectionState::PeerClosed => State::PeerClosed,
        ConnectionState::Closed => State::Closed,
        _ => State::BlockedHandshake, // Unknown variants treated as needing more data.
    };

    // Discard consumed bytes from read_buf after all borrows are released.
    read_buf.advance(discard);

    Ok(state)
}

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
    session: Rc<RefCell<Session<C>>>,
}

impl<C, Io> Clone for TlsStream<C, Io>
where
    Io: Clone,
{
    fn clone(&self) -> Self {
        Self {
            io: self.io.clone(),
            session: self.session.clone(),
        }
    }
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
    C: ProcessTlsRecords,
    Io: AsyncBufRead + AsyncBufWrite,
{
    pub async fn handshake(io: Io, conn: C) -> io::Result<Self> {
        let stream = TlsStream {
            io,
            session: Rc::new(RefCell::new(Session {
                conn,
                read_buf: Some(BytesMut::new()),
                write_buf: Some(BytesMut::new()),
                proto_write_buf: BytesMut::new(),
                pending_plaintext: BytesMut::new(),
            })),
        };
        stream._handshake().await?;
        Ok(stream)
    }

    async fn _handshake(&self) -> io::Result<()> {
        let mut session = self.session.borrow_mut();
        let mut read_buf = session.read_buf.take().expect(POLL_TO_COMPLETE);
        let mut proto_write_buf = session.proto_write_buf.split();

        let res = loop {
            let res = match process_once(&mut session.conn, &mut read_buf, &mut proto_write_buf) {
                Err(e) => Err(e),

                // Continue processing — more handshake data may follow.
                Ok(State::EncodedTlsData) => continue,

                Ok(State::TransmitTlsData) => {
                    let (res, b) = write_all_buf(&self.io, proto_write_buf).await;
                    proto_write_buf = b;
                    res
                }

                Ok(State::BlockedHandshake) => {
                    let (res, b) = read_to_buf(&self.io, read_buf).await;
                    read_buf = b;
                    res
                }

                Ok(State::WriteTraffic | State::ReadTraffic) => break Ok(()),

                Ok(State::PeerClosed | State::Closed) => {
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, "tls handshake eof"))
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

    /// Read ciphertext from IO, decrypt, and return plaintext.
    async fn read_tls(&self, plain_buf: &mut impl BoundedBufMut) -> io::Result<usize> {
        let mut session = self.session.borrow_mut();

        // Check for plaintext buffered from a previous read first.
        if !session.pending_plaintext.is_empty() {
            let dst = io_ref_mut_slice(plain_buf);
            let n = session.pending_plaintext.len().min(dst.len());
            dst[..n].copy_from_slice(&session.pending_plaintext[..n]);
            session.pending_plaintext.advance(n);
            unsafe { plain_buf.set_init(n) };
            return Ok(n);
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
                    let dst = io_ref_mut_slice(plain_buf);
                    let mut written = 0;

                    let mut err = None;
                    while let Some(res) = traffic.next_record() {
                        match res.map_err(tls_err) {
                            Ok(record) => {
                                let payload = record.payload;
                                let n = payload.len().min(dst.len() - written);
                                dst[written..written + n].copy_from_slice(&payload[..n]);
                                written += n;
                                // Buffer overflow into pending_plaintext.
                                if n < payload.len() {
                                    session_ref.pending_plaintext.extend_from_slice(&payload[n..]);
                                }
                            }
                            Err(e) => {
                                err = Some(e);
                                break;
                            }
                        }
                    }

                    drop(traffic);
                    read_buf.advance(discard);

                    if let Some(e) = err {
                        break Err(e);
                    }

                    // Empty plaintext means TLS overhead with no payload — keep going.
                    if written == 0 {
                        continue;
                    }

                    unsafe { plain_buf.set_init(written) };
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

    /// Encrypt plaintext and write ciphertext to IO.
    async fn write_tls(&self, plain: &impl BoundedBuf) -> io::Result<usize> {
        let mut session = self.session.borrow_mut();
        let mut write_buf = session.write_buf.take().expect(POLL_TO_COMPLETE);
        let plaintext = io_ref_slice(plain);

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
                    let enc_res = encrypt_to_buf(&mut traffic, plaintext, &mut write_buf);

                    if let Err(e) = enc_res {
                        break Err(e);
                    }

                    drop(session);

                    let (res, b) = write_all_buf(&self.io, write_buf).await;
                    write_buf = b;

                    session = self.session.borrow_mut();

                    break res.map(|_| plaintext.len());
                }

                Ok(ConnectionState::EncodeTlsData(mut state)) => {
                    let enc_res = encode_tls_data(&mut state, &mut write_buf);
                    drop(state);

                    if let Err(e) = enc_res {
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
}

impl<C, Io> AsyncBufRead for TlsStream<C, Io>
where
    C: ProcessTlsRecords,
    Io: AsyncBufRead + AsyncBufWrite,
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
    Io: AsyncBufRead + AsyncBufWrite,
{
    async fn write<B>(&self, buf: B) -> (io::Result<usize>, B)
    where
        B: BoundedBuf,
    {
        let res = self.write_tls(&buf).await;
        (res, buf)
    }

    fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
        self.io.shutdown(direction)
    }
}

fn io_ref_slice(buf: &impl BoundedBuf) -> &[u8] {
    // SAFETY: trust BoundedBuf implementor to provide valid pointer and length.
    unsafe { slice::from_raw_parts(buf.stable_ptr(), buf.bytes_init()) }
}

fn io_ref_mut_slice(buf: &mut impl BoundedBufMut) -> &mut [u8] {
    // SAFETY: trust BoundedBufMut implementor to provide valid pointer and capacity.
    unsafe { slice::from_raw_parts_mut(buf.stable_mut_ptr(), buf.bytes_total()) }
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
    while !buf.is_empty() {
        let (res, b) = io.write(buf).await;
        buf = b;
        match res {
            Ok(0) => return (Err(io::ErrorKind::UnexpectedEof.into()), buf),
            Ok(n) => buf.advance(n),
            Err(e) => return (Err(e), buf),
        }
    }
    (Ok(()), buf)
}

/// Encode TLS handshake data into the write buffer, resizing if needed.
fn encode_tls_data<Data>(state: &mut unbuffered::EncodeTlsData<'_, Data>, write_buf: &mut BytesMut) -> io::Result<()> {
    loop {
        let spare = write_buf.spare_capacity_mut();
        // SAFETY: encode writes into the spare capacity; we track how many bytes.
        let dst = unsafe { slice::from_raw_parts_mut(spare.as_mut_ptr().cast::<u8>(), spare.len()) };
        match state.encode(dst) {
            Ok(n) => {
                // SAFETY: encode wrote n bytes into the spare capacity.
                unsafe { write_buf.set_len(write_buf.len() + n) };
                return Ok(());
            }
            Err(unbuffered::EncodeError::InsufficientSize(unbuffered::InsufficientSizeError { required_size })) => {
                write_buf.reserve(required_size);
            }
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
        }
    }
}

/// Encrypt plaintext into the write buffer, resizing if needed.
fn encrypt_to_buf<Data>(
    traffic: &mut unbuffered::WriteTraffic<'_, Data>,
    plaintext: &[u8],
    write_buf: &mut BytesMut,
) -> io::Result<()> {
    let needed = plaintext.len() + 64;
    if write_buf.capacity() - write_buf.len() < needed {
        write_buf.reserve(needed);
    }

    loop {
        let spare = write_buf.spare_capacity_mut();
        let dst = unsafe { slice::from_raw_parts_mut(spare.as_mut_ptr().cast::<u8>(), spare.len()) };
        match traffic.encrypt(plaintext, dst) {
            Ok(n) => {
                unsafe { write_buf.set_len(write_buf.len() + n) };
                return Ok(());
            }
            Err(EncryptError::InsufficientSize(unbuffered::InsufficientSizeError { required_size })) => {
                write_buf.reserve(required_size);
            }
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
        }
    }
}

const POLL_TO_COMPLETE: &str = "previous call to future dropped before polling to completion";
