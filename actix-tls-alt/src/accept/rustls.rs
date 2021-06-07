use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
    io, mem,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use actix_server_alt::net::{AsProtocol, AsyncReadWrite, Protocol};
use actix_service_alt::{Service, ServiceFactory};
use bytes::BufMut;
use futures_task::noop_waker;
use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};
use tokio_rustls::rustls::{ServerConfig, Session};

use tokio_rustls::TlsAcceptor;

pub type RustlsConfig = Arc<ServerConfig>;

/// A wrapper type for [TlsStream](tokio_rustls::TlsStream).
///
/// This is to impl new trait for it.
pub struct TlsStream<S> {
    stream: tokio_rustls::server::TlsStream<S>,
}

impl<S> AsProtocol for TlsStream<S> {
    fn as_protocol(&self) -> Protocol {
        self.get_ref()
            .1
            .get_alpn_protocol()
            .map(|proto| {
                if proto.windows(2).any(|window| window == b"h2") {
                    Protocol::Http2
                } else {
                    Protocol::Http1Tls
                }
            })
            .unwrap_or(Protocol::Http1Tls)
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

impl<St: AsyncReadWrite> ServiceFactory<St> for TlsAcceptorService {
    type Response = TlsStream<St>;
    type Error = RustlsError;
    type Config = ();
    type Service = TlsAcceptorService;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let this = self.clone();
        async move { Ok(this) }
    }
}

impl<St: AsyncReadWrite> Service<St> for TlsAcceptorService {
    type Response = TlsStream<St>;
    type Error = RustlsError;

    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call<'c>(&'c self, io: St) -> Self::Future<'c>
    where
        St: 'c,
    {
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

impl<S: AsyncReadWrite> AsyncReadWrite for TlsStream<S> {
    type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>>;

    #[inline]
    fn ready(&mut self, interest: Interest) -> Self::ReadyFuture<'_> {
        self.get_mut().0.ready(interest)
    }

    fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize> {
        if !buf.has_remaining_mut() {
            return Ok(0);
        }

        let waker = noop_waker();
        let cx = &mut Context::from_waker(&waker);

        let n = {
            let dst = buf.chunk_mut();
            let dst = unsafe { &mut *(dst as *mut _ as *mut [mem::MaybeUninit<u8>]) };
            let mut buf = ReadBuf::uninit(dst);
            match AsyncRead::poll_read(Pin::new(self), cx, &mut buf) {
                Poll::Pending => return Err(io::Error::from(io::ErrorKind::WouldBlock)),
                Poll::Ready(res) => res?,
            };

            buf.filled().len()
        };

        // # SAFETY:
        // This is guaranteed to be the number of initialized (and read)
        // bytes due to the invariants provided by `ReadBuf::filled`.
        unsafe {
            buf.advance_mut(n);
        }

        Ok(n)
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

    #[inline]
    fn poll_read_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().0.poll_read_ready(cx)
    }

    #[inline]
    fn poll_write_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().0.poll_write_ready(cx)
    }
}

/// Collection of 'rustls' error types.
pub struct RustlsError(io::Error);

impl Debug for RustlsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl From<io::Error> for RustlsError {
    fn from(e: io::Error) -> Self {
        Self(e)
    }
}
