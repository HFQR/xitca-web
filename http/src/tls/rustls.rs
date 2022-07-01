pub(crate) type RustlsConfig = Arc<ServerConfig>;

use std::{
    convert::Infallible,
    error,
    fmt::{self, Debug, Display, Formatter},
    future::Future,
    io::{self, Write},
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_task::noop_waker;
use tokio_rustls::{
    rustls::{ServerConfig, Writer},
    TlsAcceptor,
};
use tokio_util::io::poll_read_buf;
use xitca_io::io::{AsyncIo, AsyncRead, AsyncWrite, Interest, ReadBuf, Ready, StdIoWriteAdapter};
use xitca_service::{BuildService, Service};

use crate::{bytes::BufMut, http::Version, version::AsVersion};

use super::error::TlsError;

/// A wrapper type for [TlsStream](tokio_rustls::TlsStream).
///
/// This is to impl new trait for it.
pub struct TlsStream<S> {
    stream: tokio_rustls::server::TlsStream<S>,
}

impl<S> AsVersion for TlsStream<S> {
    fn as_version(&self) -> Version {
        self.get_ref()
            .1
            .alpn_protocol()
            .map(Self::from_alpn)
            .unwrap_or(Version::HTTP_11)
    }
}

impl<S> Deref for TlsStream<S> {
    type Target = tokio_rustls::server::TlsStream<S>;

    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

impl<S> DerefMut for TlsStream<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stream
    }
}

/// Rustls Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
#[derive(Clone)]
pub struct TlsAcceptorService {
    acceptor: TlsAcceptor,
}

impl TlsAcceptorService {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self {
            acceptor: TlsAcceptor::from(config),
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

impl<St: AsyncIo> Service<St> for TlsAcceptorService {
    type Response = TlsStream<St>;
    type Error = RustlsError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn call(&self, io: St) -> Self::Future<'_> {
        async move {
            let stream = self.acceptor.accept(io).await?;
            Ok(TlsStream { stream })
        }
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
        <tokio_rustls::server::TlsStream<S>>::poll_flush(Pin::new(&mut self.get_mut().stream), cx)
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
    type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>> where Self: 'f;

    #[inline]
    fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_> {
        self.get_ref().0.ready(interest)
    }

    fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize> {
        let waker = noop_waker();
        let cx = &mut Context::from_waker(&waker);

        match poll_read_buf(Pin::new(self), cx, buf) {
            Poll::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Poll::Ready(res) => res,
        }
    }

    #[inline]
    fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write_with(self, |writer| writer.write(buf))
    }

    #[inline]
    fn try_write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        write_with(self, |writer| writer.write_vectored(bufs))
    }
}

fn write_with<S, F>(stream: &mut TlsStream<S>, func: F) -> io::Result<usize>
where
    S: AsyncIo,
    F: for<'r> FnOnce(&mut Writer<'r>) -> io::Result<usize>,
{
    let (io, conn) = stream.stream.get_mut();

    // drain remaining data left in tls buffer. this part is not included in current write.
    // (it happens from last try_write call)
    while conn.wants_write() {
        conn.write_tls(&mut StdIoWriteAdapter(io))?;
    }

    // write to tls buffer and try to send them through wire.
    let n = func(&mut conn.writer())?;

    // there is no guarantee write_tls would be able to send all the n bytes that go into tls buffer.
    // hence the previous while loop try to drain the data that try to drain the buffer.
    conn.write_tls(&mut StdIoWriteAdapter(io))?;

    // regardless write_tls written how many bytes must advance n bytes that get into tls buffer.
    Ok(n)
}

/// Collection of 'rustls' error types.
pub struct RustlsError(io::Error);

impl Debug for RustlsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl Display for RustlsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl error::Error for RustlsError {}

impl From<io::Error> for RustlsError {
    fn from(e: io::Error) -> Self {
        Self(e)
    }
}

impl From<RustlsError> for TlsError {
    fn from(e: RustlsError) -> Self {
        Self::Rustls(e)
    }
}
