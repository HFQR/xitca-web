pub(crate) use openssl::ssl::SslAcceptor as TlsAcceptor;

use std::{
    convert::Infallible,
    fmt::{self, Debug, Formatter},
    future::Future,
    io,
    pin::Pin,
    task::{ready, Context, Poll},
};

use openssl::{
    error::ErrorStack,
    ssl::{Error, ErrorCode, ShutdownResult, Ssl, SslStream},
};
use xitca_io::io::{AsyncIo, AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};
use xitca_service::Service;

use crate::{http::Version, version::AsVersion};

use super::error::TlsError;

/// A wrapper type for [SslStream].
///
/// This is to impl new trait for it.
pub struct TlsStream<Io> {
    io: SslStream<Io>,
}

impl<Io> AsVersion for TlsStream<Io> {
    fn as_version(&self) -> Version {
        self.io
            .ssl()
            .selected_alpn_protocol()
            .map(Self::from_alpn)
            .unwrap_or(Version::HTTP_11)
    }
}

#[derive(Clone)]
pub struct TlsAcceptorBuilder {
    acceptor: TlsAcceptor,
}

impl TlsAcceptorBuilder {
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Self { acceptor }
    }
}

impl<Arg> Service<Arg> for TlsAcceptorBuilder {
    type Response = TlsAcceptorService;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    fn call(&self, _: Arg) -> Self::Future<'_> {
        async {
            Ok(TlsAcceptorService {
                acceptor: self.acceptor.clone(),
            })
        }
    }
}

/// Openssl Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
pub struct TlsAcceptorService {
    acceptor: TlsAcceptor,
}

impl TlsAcceptorService {
    #[inline(never)]
    async fn accept<Io: AsyncIo>(&self, io: Io) -> Result<TlsStream<Io>, OpensslError> {
        let ctx = self.acceptor.context();
        let ssl = Ssl::new(ctx)?;
        let mut io = SslStream::new(ssl, io)?;
        let mut interest = Interest::READABLE;
        loop {
            io.get_mut().ready(interest).await?;
            match io.accept() {
                Ok(_) => return Ok(TlsStream { io }),
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    interest = Interest::READABLE;
                }
                Err(ref e) if e.code() == ErrorCode::WANT_WRITE => {
                    interest = Interest::WRITABLE;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl<Io: AsyncIo> Service<Io> for TlsAcceptorService {
    type Response = TlsStream<Io>;
    type Error = OpensslError;
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
        let this = self.get_mut();
        // copied from tokio-openssl crate.
        match this.io.shutdown() {
            Ok(ShutdownResult::Sent) | Ok(ShutdownResult::Received) => {}
            Err(ref e) if e.code() == ErrorCode::ZERO_RETURN => {}
            Err(ref e) if e.code() == ErrorCode::WANT_READ || e.code() == ErrorCode::WANT_WRITE => {
                return Poll::Pending;
            }
            Err(e) => {
                return Poll::Ready(Err(e
                    .into_io_error()
                    .unwrap_or_else(|e| io::Error::new(io::ErrorKind::Other, e))));
            }
        }

        AsyncIo::poll_shutdown(Pin::new(this.io.get_mut()), cx)
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

/// Collection of 'openssl' error types.
pub enum OpensslError {
    Io(io::Error),
    Tls(Error),
    Stack(ErrorStack),
}

impl Debug for OpensslError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{:?}", e),
            Self::Tls(ref e) => write!(f, "{:?}", e),
            Self::Stack(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl From<io::Error> for OpensslError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ErrorStack> for OpensslError {
    fn from(e: ErrorStack) -> Self {
        Self::Stack(e)
    }
}

impl From<Error> for OpensslError {
    fn from(e: Error) -> Self {
        Self::Tls(e)
    }
}

impl From<OpensslError> for TlsError {
    fn from(e: OpensslError) -> Self {
        Self::Openssl(e)
    }
}
