pub(crate) type RustlsConfig = Arc<ServerConfig>;

use std::{
    convert::Infallible,
    error, fmt,
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_core::ready;
use rustls::{Error, ServerConfig, ServerConnection, StreamOwned};
use xitca_io::io::{AsyncIo, AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};
use xitca_service::{BuildService, Service};

use crate::{http::Version, version::AsVersion};

use super::error::TlsError;

/// A stream managed by rustls for tls read/write.
pub struct TlsStream<Io>
where
    Io: AsyncIo,
{
    io: StreamOwned<ServerConnection, Io>,
}

impl<Io> AsVersion for TlsStream<Io>
where
    Io: AsyncIo,
{
    fn as_version(&self) -> Version {
        self.io
            .conn
            .alpn_protocol()
            .map(Self::from_alpn)
            .unwrap_or(Version::HTTP_11)
    }
}

/// Rustls Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
#[derive(Clone)]
pub struct TlsAcceptorService {
    config: Arc<ServerConfig>,
}

impl TlsAcceptorService {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self { config }
    }

    #[inline(never)]
    async fn accept<Io: AsyncIo>(&self, mut io: Io) -> Result<TlsStream<Io>, RustlsError> {
        let mut conn = ServerConnection::new(self.config.clone())?;

        loop {
            let interest = match conn.complete_io(&mut io) {
                Ok(_) => {
                    return Ok(TlsStream {
                        io: StreamOwned::new(conn, io),
                    })
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => match (conn.wants_read(), conn.wants_write()) {
                    (true, true) => Interest::READABLE | Interest::WRITABLE,
                    (true, false) => Interest::READABLE,
                    (false, true) => Interest::WRITABLE,
                    (false, false) => unreachable!(),
                },
                Err(e) => return Err(e.into()),
            };

            io.ready(interest).await?;
        }
    }
}

impl BuildService for TlsAcceptorService {
    type Service = TlsAcceptorService;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, _: ()) -> Self::Future {
        let this = self.clone();
        async { Ok(this) }
    }
}

impl<Io: AsyncIo> Service<Io> for TlsAcceptorService {
    type Response = TlsStream<Io>;
    type Error = RustlsError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, io: Io) -> Self::Future<'_> {
        self.accept(io)
    }
}

impl<Io: AsyncIo> AsyncIo for TlsStream<Io> {
    type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>> where Self: 'f;

    #[inline]
    fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_> {
        self.io.get_ref().ready(interest)
    }

    #[inline]
    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        self.io.get_ref().poll_ready(interest, cx)
    }

    fn is_vectored_write(&self) -> bool {
        self.io.get_ref().is_vectored_write()
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncIo::poll_shutdown(Pin::new(self.get_mut().io.get_mut()), cx)
    }
}

impl<Io: AsyncIo> io::Read for TlsStream<Io> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.io, buf)
    }
}

impl<Io: AsyncIo> io::Write for TlsStream<Io> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(&mut self.io, buf)
    }

    #[inline]
    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        io::Write::write_vectored(&mut self.io, bufs)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.io)
    }
}

impl<Io> AsyncRead for TlsStream<Io>
where
    Io: AsyncIo,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.io.get_ref().poll_ready(Interest::READABLE, cx))?;
        match io::Read::read(this, buf.initialize_unfilled()) {
            Ok(n) => {
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<Io> AsyncWrite for TlsStream<Io>
where
    Io: AsyncIo,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.io.get_ref().poll_ready(Interest::WRITABLE, cx))?;
        match io::Write::write(this, buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.io.get_ref().poll_ready(Interest::WRITABLE, cx))?;
        match io::Write::flush(this) {
            Ok(_) => Poll::Ready(Ok(())),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncIo::poll_shutdown(self, cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.io.get_ref().poll_ready(Interest::WRITABLE, cx))?;
        match io::Write::write_vectored(this, bufs) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.io.get_ref().is_vectored_write()
    }
}

/// Collection of 'rustls' error types.
pub enum RustlsError {
    Io(io::Error),
    Tls(Error),
}

impl fmt::Debug for RustlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{:?}", e),
            Self::Tls(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl fmt::Display for RustlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{}", e),
            Self::Tls(ref e) => write!(f, "{}", e),
        }
    }
}

impl error::Error for RustlsError {}

impl From<io::Error> for RustlsError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Error> for RustlsError {
    fn from(e: Error) -> Self {
        Self::Tls(e)
    }
}

impl From<RustlsError> for TlsError {
    fn from(e: RustlsError) -> Self {
        Self::Rustls(e)
    }
}
