use std::{
    io::{self, IoSlice},
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(feature = "openssl")]
use tokio_openssl::SslStream;

#[doc(hidden)]
pub enum TlsStream<S> {
    NoOp(S),
    #[cfg(feature = "openssl")]
    Openssl(SslStream<S>),
}

impl<S> AsyncRead for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::NoOp(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl<S> AsyncWrite for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::NoOp(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::NoOp(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::NoOp(s) => Pin::new(s).poll_shutdown(cx),
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
            Self::NoOp(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(feature = "openssl")]
            Self::Openssl(s) => Pin::new(s).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match *self {
            Self::NoOp(ref s) => s.is_write_vectored(),
            #[cfg(feature = "openssl")]
            Self::Openssl(ref s) => s.is_write_vectored(),
        }
    }
}
