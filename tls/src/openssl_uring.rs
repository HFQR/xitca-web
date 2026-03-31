use core::cell::RefCell;

use std::{
    io::{self, Read, Write},
    net::Shutdown,
    rc::Rc,
};

pub use openssl::*;

use openssl::ssl::{ErrorCode, Ssl, SslRef, SslStream};

use xitca_io::{
    bytes::{Buf, BytesMut},
    io_uring::{AsyncBufRead, AsyncBufWrite, BoundedBuf, BoundedBufMut},
};

/// A TLS stream backed by OpenSSL that implements completion-based IO traits.
///
/// Uses a sync-to-async bridge: OpenSSL reads/writes against in-memory buffers
/// ([`SyncStream`]), and the actual socket IO is performed asynchronously.
///
/// Supports concurrent read/write: `ssl_read`/`ssl_write` are synchronous
/// operations on in-memory buffers. The `RefCell` borrow is dropped before
/// any async socket IO, allowing the other path to proceed.
///
/// # Panics
/// Each async read/write operation must be polled to completion. Dropping a future before it
/// completes will leave internal buffers in a taken state, causing the next call to panic.
/// Concurrent reads or concurrent writes (two reads at the same time, etc.) will also panic.
pub struct TlsStream<Io> {
    io: Io,
    session: Rc<RefCell<Session>>,
}

struct Session {
    ssl: SslStream<SyncStream>,
    /// Protocol data produced by read path (key updates, alerts).
    /// Flushed by write path before sending application data.
    proto_write_buf: BytesMut,
}

/// Synchronous stream adapter that OpenSSL reads from / writes to.
///
/// Buffers are `Option<BytesMut>` to detect concurrent misuse: each path
/// takes its buffer before async IO and replaces it after. A second concurrent
/// operation on the same path will find `None` and panic.
///
/// `Read` pulls ciphertext from `read_buf`. `Write` appends ciphertext to
/// `write_buf`. Returns `WouldBlock` when `read_buf` is empty to signal
/// OpenSSL to yield.
struct SyncStream {
    /// Ciphertext from the socket, consumed by OpenSSL during `ssl_read`.
    read_buf: Option<BytesMut>,
    /// Ciphertext produced by OpenSSL, to be sent to the socket.
    write_buf: Option<BytesMut>,
}

impl Read for SyncStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read_buf = self.read_buf.as_mut().expect(POLL_TO_COMPLETE);
        if read_buf.is_empty() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = buf.len().min(read_buf.len());
        buf[..n].copy_from_slice(&read_buf[..n]);
        read_buf.advance(n);
        Ok(n)
    }
}

impl Write for SyncStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let write_buf = self.write_buf.as_mut().expect(POLL_TO_COMPLETE);
        write_buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<Io> TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    /// Perform a TLS handshake as the server side.
    pub async fn accept(ssl: Ssl, io: Io) -> Result<Self, Error> {
        let stream = Self::new(ssl, io)?;
        stream.handshake(|ssl| ssl.accept()).await?;
        Ok(stream)
    }

    /// Perform a TLS handshake as the client side.
    pub async fn connect(ssl: Ssl, io: Io) -> Result<Self, Error> {
        let stream = Self::new(ssl, io)?;
        stream.handshake(|ssl| ssl.connect()).await?;
        Ok(stream)
    }

    fn new(ssl: Ssl, io: Io) -> Result<Self, Error> {
        let sync_stream = SyncStream {
            read_buf: Some(BytesMut::new()),
            write_buf: Some(BytesMut::new()),
        };
        let ssl_stream = SslStream::new(ssl, sync_stream)?;

        Ok(TlsStream {
            io,
            session: Rc::new(RefCell::new(Session {
                ssl: ssl_stream,
                proto_write_buf: BytesMut::new(),
            })),
        })
    }

    /// Acquire a reference to the `SslRef` for inspecting the session.
    pub fn session(&self) -> impl core::ops::Deref<Target = SslRef> + '_ {
        std::cell::Ref::map(self.session.borrow(), |s| s.ssl.ssl())
    }

    async fn handshake<F>(&self, mut func: F) -> Result<(), Error>
    where
        F: FnMut(&mut SslStream<SyncStream>) -> Result<(), openssl::ssl::Error>,
    {
        let mut session = self.session.borrow_mut();

        loop {
            match func(&mut session.ssl) {
                Ok(()) => {
                    // Flush any remaining handshake data.
                    let sync = session.ssl.get_mut();
                    if sync.write_buf.as_ref().is_some_and(|b| !b.is_empty()) {
                        let mut write_buf = sync.write_buf.take().expect(POLL_TO_COMPLETE);
                        drop(session);
                        let (res, b) = flush_write_buf(&self.io, write_buf).await;
                        write_buf = b;
                        session = self.session.borrow_mut();
                        session.ssl.get_mut().write_buf = Some(write_buf);
                        res?;
                    }
                    return Ok(());
                }
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    // Flush outgoing handshake data first.
                    let sync = session.ssl.get_mut();
                    if sync.write_buf.as_ref().is_some_and(|b| !b.is_empty()) {
                        let mut write_buf = sync.write_buf.take().expect(POLL_TO_COMPLETE);
                        drop(session);
                        let (res, b) = flush_write_buf(&self.io, write_buf).await;
                        write_buf = b;
                        session = self.session.borrow_mut();
                        session.ssl.get_mut().write_buf = Some(write_buf);
                        res?;
                    }

                    // Read more ciphertext from the socket.
                    let mut read_buf = session.ssl.get_mut().read_buf.take().expect(POLL_TO_COMPLETE);
                    drop(session);
                    let (res, b) = read_to_buf(&self.io, read_buf).await;
                    read_buf = b;
                    session = self.session.borrow_mut();
                    session.ssl.get_mut().read_buf = Some(read_buf);
                    res?;
                }
                Err(ref e) if e.code() == ErrorCode::WANT_WRITE => {
                    // Flush outgoing handshake data.
                    let mut write_buf = session.ssl.get_mut().write_buf.take().expect(POLL_TO_COMPLETE);
                    drop(session);
                    let (res, b) = flush_write_buf(&self.io, write_buf).await;
                    write_buf = b;
                    session = self.session.borrow_mut();
                    session.ssl.get_mut().write_buf = Some(write_buf);
                    res?;
                }
                Err(e) => return Err(Error::Tls(e)),
            }
        }
    }

    /// Read plaintext by decrypting ciphertext from the socket.
    ///
    /// Protocol data produced during read (key updates, alerts) is buffered
    /// in `proto_write_buf` and flushed by the next `write_tls` call.
    async fn read_tls(&self, plain_buf: &mut impl BoundedBufMut) -> io::Result<usize> {
        let mut session = self.session.borrow_mut();

        loop {
            let dst = io_ref_mut_slice(plain_buf);
            match session.ssl.ssl_read(dst) {
                Ok(n) => {
                    unsafe { plain_buf.set_init(n) };

                    // Drain protocol data into proto_write_buf for write path to flush.
                    drain_proto_write(&mut session);

                    return Ok(n);
                }
                Err(ref e) if e.code() == ErrorCode::ZERO_RETURN => return Ok(0),
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    // Drain protocol data into proto_write_buf.
                    drain_proto_write(&mut session);

                    // Read more ciphertext from the socket.
                    let mut read_buf = session.ssl.get_mut().read_buf.take().expect(POLL_TO_COMPLETE);
                    drop(session);
                    let (res, b) = read_to_buf(&self.io, read_buf).await;
                    read_buf = b;
                    session = self.session.borrow_mut();
                    session.ssl.get_mut().read_buf = Some(read_buf);
                    res?;
                }
                Err(e) => {
                    return Err(e
                        .into_io_error()
                        .unwrap_or_else(|e| io::Error::new(io::ErrorKind::Other, e)));
                }
            }
        }
    }

    /// Encrypt plaintext and write ciphertext to the socket.
    ///
    /// Flushes any protocol data buffered by the read path before writing.
    async fn write_tls(&self, plain: &impl BoundedBuf) -> io::Result<usize> {
        let mut session = self.session.borrow_mut();
        let plaintext = io_ref_slice(plain);

        // Flush protocol data from read path into write_buf.
        let session_ref = &mut *session;
        if !session_ref.proto_write_buf.is_empty() {
            let write_buf = session_ref.ssl.get_mut().write_buf.as_mut().expect(POLL_TO_COMPLETE);
            write_buf.extend_from_slice(&session_ref.proto_write_buf);
            session_ref.proto_write_buf.clear();
        }

        loop {
            match session.ssl.ssl_write(plaintext) {
                Ok(n) => {
                    // Flush ciphertext to the socket.
                    let mut write_buf = session.ssl.get_mut().write_buf.take().expect(POLL_TO_COMPLETE);
                    drop(session);
                    let (res, b) = flush_write_buf(&self.io, write_buf).await;
                    write_buf = b;
                    session = self.session.borrow_mut();
                    session.ssl.get_mut().write_buf = Some(write_buf);
                    res?;

                    return Ok(n);
                }
                Err(ref e) if e.code() == ErrorCode::WANT_WRITE => {
                    // Flush and retry.
                    let mut write_buf = session.ssl.get_mut().write_buf.take().expect(POLL_TO_COMPLETE);
                    drop(session);
                    let (res, b) = flush_write_buf(&self.io, write_buf).await;
                    write_buf = b;
                    session = self.session.borrow_mut();
                    session.ssl.get_mut().write_buf = Some(write_buf);
                    res?;
                }
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    // Renegotiation — flush then read before retrying.
                    let sync = session.ssl.get_mut();
                    if sync.write_buf.as_ref().is_some_and(|b| !b.is_empty()) {
                        let mut write_buf = sync.write_buf.take().expect(POLL_TO_COMPLETE);
                        drop(session);
                        let (res, b) = flush_write_buf(&self.io, write_buf).await;
                        write_buf = b;
                        session = self.session.borrow_mut();
                        session.ssl.get_mut().write_buf = Some(write_buf);
                        res?;
                    }

                    let mut read_buf = session.ssl.get_mut().read_buf.take().expect(POLL_TO_COMPLETE);
                    drop(session);
                    let (res, b) = read_to_buf(&self.io, read_buf).await;
                    read_buf = b;
                    session = self.session.borrow_mut();
                    session.ssl.get_mut().read_buf = Some(read_buf);
                    res?;
                }
                Err(e) => {
                    return Err(e
                        .into_io_error()
                        .unwrap_or_else(|e| io::Error::new(io::ErrorKind::Other, e)));
                }
            }
        }
    }
}

/// Move protocol ciphertext produced during `ssl_read` to `proto_write_buf`.
/// The write path will flush it to the socket.
fn drain_proto_write(session: &mut Session) {
    let sync = session.ssl.get_mut();
    if let Some(write_buf) = sync.write_buf.as_mut() {
        if !write_buf.is_empty() {
            session.proto_write_buf.extend_from_slice(write_buf);
            write_buf.clear();
        }
    }
}

impl<Io> AsyncBufRead for TlsStream<Io>
where
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

impl<Io> AsyncBufWrite for TlsStream<Io>
where
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
    unsafe { core::slice::from_raw_parts(buf.stable_ptr(), buf.bytes_init()) }
}

fn io_ref_mut_slice(buf: &mut impl BoundedBufMut) -> &mut [u8] {
    unsafe { core::slice::from_raw_parts_mut(buf.stable_mut_ptr(), buf.bytes_total()) }
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
async fn flush_write_buf(io: &impl AsyncBufWrite, mut buf: BytesMut) -> (io::Result<()>, BytesMut) {
    let (res, b) = xitca_io::io_uring::write_all(io, buf).await;
    buf = b;
    if res.is_ok() {
        buf.clear();
    }
    (res, buf)
}

const POLL_TO_COMPLETE: &str = "previous call to future dropped before polling to completion";

/// Collection of OpenSSL error types.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Tls(openssl::ssl::Error),
}

impl From<openssl::error::ErrorStack> for Error {
    fn from(e: openssl::error::ErrorStack) -> Self {
        Self::Tls(e.into())
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => core::fmt::Display::fmt(e, f),
            Self::Tls(e) => core::fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for Error {}
