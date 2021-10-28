use std::{
    io::{self, IoSlice},
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(feature = "openssl")]
use tokio_openssl::SslStream as OpensslStream;

#[cfg(feature = "rustls")]
use tokio_rustls::client::TlsStream as RustlsStream;

#[doc(hidden)]
pub enum TlsStream<S> {
    Boxed(Box<dyn Io>),
    _Phantom(PhantomData<S>),
    #[cfg(feature = "openssl")]
    Openssl(OpensslStream<S>),
    #[cfg(feature = "rustls")]
    Rustls(RustlsStream<S>),
}

impl<S> From<Box<dyn Io>> for TlsStream<S> {
    fn from(stream: Box<dyn Io>) -> Self {
        Self::Boxed(stream)
    }
}

#[cfg(feature = "openssl")]
impl<S> From<OpensslStream<S>> for TlsStream<S> {
    fn from(stream: OpensslStream<S>) -> Self {
        Self::Openssl(stream)
    }
}

#[cfg(feature = "rustls")]
impl<S> From<RustlsStream<S>> for TlsStream<S> {
    fn from(stream: RustlsStream<S>) -> Self {
        Self::Rustls(stream)
    }
}

/// A trait impl for all types that impl [AsyncRead], [AsyncWrite], [Send] and [Unpin].
/// Enabling `Box<dyn Io>` trait object usage.
pub trait Io: AsyncRead + AsyncWrite + Send + Unpin {}

impl<S> Io for S where S: AsyncRead + AsyncWrite + Send + Unpin {}

#[allow(unused_variables)]
impl<S> AsyncRead for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Boxed(io) => Pin::new(io.as_mut()).poll_read(cx, buf),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

#[allow(unused_variables)]
impl<S> AsyncWrite for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Boxed(io) => Pin::new(io.as_mut()).poll_write(cx, buf),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Boxed(io) => Pin::new(io.as_mut()).poll_flush(cx),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Boxed(io) => Pin::new(io.as_mut()).poll_shutdown(cx),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Boxed(io) => Pin::new(io.as_mut()).poll_write_vectored(cx, bufs),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => Pin::new(s).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match *self {
            Self::Boxed(ref io) => io.is_write_vectored(),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(ref s) => s.is_write_vectored(),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref s) => s.is_write_vectored(),
        }
    }
}
