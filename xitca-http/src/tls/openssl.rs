pub(crate) use openssl_crate::ssl::SslAcceptor as TlsAcceptor;

use std::{
    fmt::{self, Debug, Formatter},
    future::{self, ready, Future},
    io,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

use futures_task::noop_waker;
use openssl_crate::{
    error::{Error, ErrorStack},
    ssl::{self, Ssl},
};
use tokio_util::io::poll_read_buf;
use xitca_io::io::{AsyncIo, AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};
use xitca_service::{Service, ServiceFactory};

use crate::{bytes::BufMut, http::Version, version::AsVersion};

use super::error::TlsError;

/// A wrapper type for [SslStream](tokio_openssl::SslStream).
///
/// This is to impl new trait for it.
pub struct TlsStream<S> {
    stream: tokio_openssl::SslStream<S>,
}

impl<S> AsVersion for TlsStream<S> {
    fn as_version(&self) -> Version {
        self.ssl()
            .selected_alpn_protocol()
            .map(Self::from_alpn)
            .unwrap_or(Version::HTTP_11)
    }
}

impl<S> Deref for TlsStream<S> {
    type Target = tokio_openssl::SslStream<S>;

    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

impl<S> DerefMut for TlsStream<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stream
    }
}

/// Openssl Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
#[derive(Clone)]
pub struct TlsAcceptorService {
    acceptor: TlsAcceptor,
}

impl TlsAcceptorService {
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Self { acceptor }
    }
}

impl<St: AsyncIo> ServiceFactory<St> for TlsAcceptorService {
    type Response = TlsStream<St>;
    type Error = OpensslError;
    type Config = ();
    type Service = TlsAcceptorService;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let this = self.clone();
        async move { Ok(this) }
    }
}

impl<St: AsyncIo> Service<St> for TlsAcceptorService {
    type Response = TlsStream<St>;
    type Error = OpensslError;

    type Ready<'f> = future::Ready<Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        ready(Ok(()))
    }

    fn call(&self, io: St) -> Self::Future<'_> {
        async move {
            let ctx = self.acceptor.context();
            let ssl = Ssl::new(ctx)?;
            let mut stream = tokio_openssl::SslStream::new(ssl, io)?;
            Pin::new(&mut stream).accept().await?;
            Ok(TlsStream { stream })
        }
    }
}

/// Collection of 'openssl' error types.
pub enum OpensslError {
    Tls(ssl::Error),
    Single(Error),
    Stack(ErrorStack),
}

impl Debug for OpensslError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Tls(ref e) => write!(f, "{:?}", e),
            Self::Single(ref e) => write!(f, "{:?}", e),
            Self::Stack(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl From<Error> for OpensslError {
    fn from(e: Error) -> Self {
        Self::Single(e)
    }
}

impl From<ErrorStack> for OpensslError {
    fn from(e: ErrorStack) -> Self {
        Self::Stack(e)
    }
}

impl From<ssl::Error> for OpensslError {
    fn from(e: ssl::Error) -> Self {
        Self::Tls(e)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for TlsStream<S> {
    #[inline]
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.get_mut().stream), cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for TlsStream<S> {
    #[inline]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.get_mut().stream), cx, buf)
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.get_mut().stream), cx)
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.get_mut().stream), cx)
    }

    #[inline]
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write_vectored(Pin::new(&mut self.get_mut().stream), cx, bufs)
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }
}

impl<S: AsyncIo> AsyncIo for TlsStream<S> {
    type ReadyFuture<'f>
    where
        Self: 'f,
    = impl Future<Output = io::Result<Ready>>;

    #[inline]
    fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_> {
        self.get_ref().ready(interest)
    }

    fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize> {
        let waker = noop_waker();
        let cx = &mut Context::from_waker(&waker);

        match poll_read_buf(Pin::new(self), cx, buf) {
            Poll::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Poll::Ready(res) => res,
        }
    }

    fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let waker = noop_waker();
        let cx = &mut Context::from_waker(&waker);

        match AsyncWrite::poll_write(Pin::new(self), cx, buf) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
        }
    }

    fn try_write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        let waker = noop_waker();
        let cx = &mut Context::from_waker(&waker);

        match AsyncWrite::poll_write_vectored(Pin::new(self), cx, bufs) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
        }
    }
}

impl From<OpensslError> for TlsError {
    fn from(e: OpensslError) -> Self {
        Self::Openssl(e)
    }
}
