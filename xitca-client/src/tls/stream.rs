use std::{
    io::{self, IoSlice},
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(feature = "openssl")]
use tokio_openssl::SslStream;

#[doc(hidden)]
pub enum TlsStream<S> {
    Boxed(Box<dyn Io>),
    _Phantom(PhantomData<S>),
    #[cfg(feature = "openssl")]
    Openssl(SslStream<S>),
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
            Self::Boxed(io) => Pin::new(io).poll_read(cx, buf),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_read(cx, buf),
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
            Self::Boxed(io) => Pin::new(io).poll_write(cx, buf),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Boxed(io) => Pin::new(io).poll_flush(cx),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Boxed(io) => Pin::new(io).poll_shutdown(cx),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_shutdown(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Boxed(io) => Pin::new(io).poll_write_vectored(cx, bufs),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match *self {
            Self::Boxed(ref io) => io.is_write_vectored(),
            Self::_Phantom(_) => unreachable!(),
            #[cfg(feature = "openssl")]
            Self::Openssl(ref s) => s.is_write_vectored(),
        }
    }
}
