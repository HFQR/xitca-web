pub(crate) use tokio_native_tls::native_tls::{Error as NativeTlsError, TlsAcceptor};

use std::{
    future::Future,
    io,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

use actix_server_alt::net::AsyncReadWrite;
use actix_service_alt::{Service, ServiceFactory};
use bytes::BufMut;
use futures_task::noop_waker;
use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};
use tokio_util::io::poll_read_buf;

use crate::protocol::{AsProtocol, Protocol};

use super::error::TlsError;

/// A wrapper type for [TlsStream](tokio_native_tls::TlsStream).
///
/// This is to impl new trait for it.
pub struct TlsStream<S> {
    stream: tokio_native_tls::TlsStream<S>,
}

impl<S> AsProtocol for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn as_protocol(&self) -> Protocol {
        self.get_ref()
            .negotiated_alpn()
            .ok()
            .and_then(|proto| proto)
            .map(Self::from_alpn)
            .unwrap_or(Protocol::Http1Tls)
    }
}

impl<S> Deref for TlsStream<S> {
    type Target = tokio_native_tls::TlsStream<S>;

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
    acceptor: tokio_native_tls::TlsAcceptor,
}

impl TlsAcceptorService {
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Self {
            acceptor: tokio_native_tls::TlsAcceptor::from(acceptor),
        }
    }
}

impl<St: AsyncReadWrite> ServiceFactory<St> for TlsAcceptorService {
    type Response = TlsStream<St>;
    type Error = NativeTlsError;
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
    type Error = NativeTlsError;

    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

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
    fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_> {
        self.get_ref().get_ref().get_ref().ready(interest)
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

impl From<NativeTlsError> for TlsError {
    fn from(e: NativeTlsError) -> Self {
        Self::NativeTls(e)
    }
}
