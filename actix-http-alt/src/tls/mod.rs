//! actix-http-alt treat all connection as running on top of tls and would try to do tls
//! accept on all of them.
//!
//! For plain Tcp and Unix sockets connection a dummy Tls acceptor and tls stream type
//! is used.

#[cfg(feature = "openssl")]
pub(crate) mod openssl;
#[cfg(feature = "rustls")]
pub(crate) mod rustls;

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use actix_server_alt::net::{AsProtocol, AsyncReadWrite, Protocol, Stream as ServerStream};
use actix_service_alt::{Service, ServiceFactory};
use bytes::BufMut;
use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};

use super::error::HttpServiceError;

/// A NoOp Tls Acceptor pass through input Stream type.
#[derive(Copy, Clone)]
pub struct NoOpTlsAcceptorService;

impl<St> ServiceFactory<St> for NoOpTlsAcceptorService {
    type Response = St;
    type Error = HttpServiceError;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async move { Ok(Self) }
    }
}

impl<St> Service<St> for NoOpTlsAcceptorService {
    type Response = St;
    type Error = HttpServiceError;

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
        async move { Ok(io) }
    }
}

/// A collection of tls acceptor services.
#[derive(Clone)]
pub enum TlsAcceptorService {
    NoOp(NoOpTlsAcceptorService),
    #[cfg(feature = "openssl")]
    OpenSsl(self::openssl::TlsAcceptorService),
    #[cfg(feature = "rustls")]
    Rustls(self::rustls::TlsAcceptorService),
}

impl TlsAcceptorService {
    pub fn new() -> Self {
        Self::NoOp(NoOpTlsAcceptorService)
    }
}

impl Default for TlsAcceptorService {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceFactory<ServerStream> for TlsAcceptorService {
    type Response = TlsStream;
    type Error = HttpServiceError;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let this = self.clone();
        async move { Ok(this) }
    }
}

impl Service<ServerStream> for TlsAcceptorService {
    type Response = TlsStream;
    type Error = HttpServiceError;

    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        match *self {
            Self::NoOp(ref tls) => <NoOpTlsAcceptorService as Service<ServerStream>>::poll_ready(tls, cx),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(ref tls) => <self::openssl::TlsAcceptorService as Service<ServerStream>>::poll_ready(tls, cx)
                .map_err(HttpServiceError::from),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref tls) => <self::rustls::TlsAcceptorService as Service<ServerStream>>::poll_ready(tls, cx)
                .map_err(HttpServiceError::from),
        }
    }

    #[inline]
    fn call<'c>(&'c self, stream: ServerStream) -> Self::Future<'c>
    where
        ServerStream: 'c,
    {
        async move {
            match *self {
                Self::NoOp(ref tls) => {
                    let stream = tls.call(stream).await?;
                    Ok(TlsStream::NoOp(stream))
                }
                #[cfg(feature = "openssl")]
                Self::OpenSsl(ref tls) => {
                    let stream = tls.call(stream).await?;
                    Ok(TlsStream::OpenSsl(stream))
                }
                #[cfg(feature = "rustls")]
                Self::Rustls(ref tls) => {
                    let stream = tls.call(stream).await?;
                    Ok(TlsStream::Rustls(stream))
                }
            }
        }
    }
}

/// a collection of streams after successful tls handshake.
pub enum TlsStream {
    NoOp(ServerStream),
    #[cfg(feature = "openssl")]
    OpenSsl(self::openssl::TlsStream<ServerStream>),
    #[cfg(feature = "rustls")]
    Rustls(self::rustls::TlsStream<ServerStream>),
}

impl AsProtocol for TlsStream {
    #[inline]
    fn as_protocol(&self) -> Protocol {
        match *self {
            Self::NoOp(ref tls) => tls.as_protocol(),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(ref tls) => tls.as_protocol(),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref tls) => tls.as_protocol(),
        }
    }
}

impl AsyncRead for TlsStream {
    #[inline]
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::NoOp(tls) => Pin::new(tls).poll_read(cx, buf),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(tls) => Pin::new(tls).poll_read(cx, buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(tls) => Pin::new(tls).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TlsStream {
    #[inline]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::NoOp(tls) => Pin::new(tls).poll_write(cx, buf),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(tls) => Pin::new(tls).poll_write(cx, buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(tls) => Pin::new(tls).poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::NoOp(tls) => Pin::new(tls).poll_flush(cx),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(tls) => Pin::new(tls).poll_flush(cx),
            #[cfg(feature = "rustls")]
            Self::Rustls(tls) => Pin::new(tls).poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::NoOp(tls) => Pin::new(tls).poll_shutdown(cx),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(tls) => Pin::new(tls).poll_shutdown(cx),
            #[cfg(feature = "rustls")]
            Self::Rustls(tls) => Pin::new(tls).poll_shutdown(cx),
        }
    }

    #[inline]
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::NoOp(tls) => Pin::new(tls).poll_write_vectored(cx, bufs),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(tls) => Pin::new(tls).poll_write_vectored(cx, bufs),
            #[cfg(feature = "rustls")]
            Self::Rustls(tls) => Pin::new(tls).poll_write_vectored(cx, bufs),
        }
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        match *self {
            Self::NoOp(ref tls) => tls.is_write_vectored(),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(ref tls) => tls.is_write_vectored(),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref tls) => tls.is_write_vectored(),
        }
    }
}

impl AsyncReadWrite for TlsStream {
    type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>>;

    #[inline]
    fn ready(&mut self, interest: Interest) -> Self::ReadyFuture<'_> {
        async move {
            match *self {
                Self::NoOp(ref mut tls) => tls.ready(interest).await,
                #[cfg(feature = "openssl")]
                Self::OpenSsl(ref mut tls) => tls.ready(interest).await,
                #[cfg(feature = "rustls")]
                Self::Rustls(ref mut tls) => tls.ready(interest).await,
            }
        }
    }

    #[inline]
    fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize> {
        match *self {
            Self::NoOp(ref mut tls) => tls.try_read_buf(buf),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(ref mut tls) => tls.try_read_buf(buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref mut tls) => tls.try_read_buf(buf),
        }
    }

    #[inline]
    fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Self::NoOp(ref mut tls) => tls.try_write(buf),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(ref mut tls) => tls.try_write(buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref mut tls) => tls.try_write(buf),
        }
    }

    #[inline]
    fn try_write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        match *self {
            Self::NoOp(ref mut tls) => tls.try_write_vectored(bufs),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(ref mut tls) => tls.try_write_vectored(bufs),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref mut tls) => tls.try_write_vectored(bufs),
        }
    }

    #[inline]
    fn poll_read_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match *self {
            Self::NoOp(ref mut tls) => tls.poll_read_ready(cx),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(ref mut tls) => tls.poll_read_ready(cx),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref mut tls) => tls.poll_read_ready(cx),
        }
    }

    #[inline]
    fn poll_write_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match *self {
            Self::NoOp(ref mut tls) => tls.poll_write_ready(cx),
            #[cfg(feature = "openssl")]
            Self::OpenSsl(ref mut tls) => tls.poll_write_ready(cx),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref mut tls) => tls.poll_write_ready(cx),
        }
    }
}
