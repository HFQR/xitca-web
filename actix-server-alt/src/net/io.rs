use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::BufMut;
use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};

/// A wrapper trait for an AsyncRead/AsyncWrite tokio type with additional methods.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin {
    type ReadyFuture<'f>: Future<Output = io::Result<Ready>>;

    fn ready(&mut self, interest: Interest) -> Self::ReadyFuture<'_>;

    fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize>;

    fn try_write(&mut self, buf: &[u8]) -> io::Result<usize>;

    fn try_write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize>;

    fn poll_read_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;

    fn poll_write_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
}

macro_rules! basic_impl {
    ($ty: ty) => {
        impl AsyncReadWrite for $ty {
            type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>>;

            #[inline]
            fn ready(&mut self, interest: Interest) -> Self::ReadyFuture<'_> {
                Self::ready(self, interest)
            }

            #[inline]
            fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize> {
                Self::try_read_buf(self, buf)
            }

            #[inline]
            fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
                Self::try_write(self, buf)
            }

            #[inline]
            fn try_write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
                Self::try_write_vectored(self, bufs)
            }

            #[inline]
            fn poll_read_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Self::poll_read_ready(self, cx)
            }

            #[inline]
            fn poll_write_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Self::poll_write_ready(self, cx)
            }
        }
    };
}

basic_impl!(super::TcpStream);

#[cfg(unix)]
basic_impl!(super::UnixStream);

impl AsyncRead for super::Stream {
    #[inline]
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(tcp) => Pin::new(tcp).poll_read(cx, buf),
            #[cfg(unix)]
            Self::Unix(unix) => Pin::new(unix).poll_read(cx, buf),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }
}

impl AsyncWrite for super::Stream {
    #[inline]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(tcp) => Pin::new(tcp).poll_write(cx, buf),
            #[cfg(unix)]
            Self::Unix(unix) => Pin::new(unix).poll_write(cx, buf),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(tcp) => Pin::new(tcp).poll_flush(cx),
            #[cfg(unix)]
            Self::Unix(unix) => Pin::new(unix).poll_flush(cx),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(tcp) => Pin::new(tcp).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(unix) => Pin::new(unix).poll_shutdown(cx),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }

    #[inline]
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(tcp) => Pin::new(tcp).poll_write_vectored(cx, bufs),
            #[cfg(unix)]
            Self::Unix(unix) => Pin::new(unix).poll_write_vectored(cx, bufs),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        match *self {
            Self::Tcp(ref tcp) => tcp.is_write_vectored(),
            #[cfg(unix)]
            Self::Unix(ref unix) => unix.is_write_vectored(),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }
}

impl AsyncReadWrite for super::Stream {
    type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>>;

    #[inline]
    fn ready(&mut self, interest: Interest) -> Self::ReadyFuture<'_> {
        async move {
            match *self {
                Self::Tcp(ref mut tcp) => tcp.ready(interest).await,
                #[cfg(unix)]
                Self::Unix(ref mut unix) => unix.ready(interest).await,
                #[cfg(feature = "http3")]
                Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
            }
        }
    }

    #[inline]
    fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize> {
        match *self {
            Self::Tcp(ref mut tcp) => tcp.try_read_buf(buf),
            #[cfg(unix)]
            Self::Unix(ref mut unix) => unix.try_read_buf(buf),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }

    #[inline]
    fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Self::Tcp(ref mut tcp) => tcp.try_write(buf),
            #[cfg(unix)]
            Self::Unix(ref mut unix) => unix.try_write(buf),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }

    #[inline]
    fn try_write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        match *self {
            Self::Tcp(ref mut tcp) => tcp.try_write_vectored(bufs),
            #[cfg(unix)]
            Self::Unix(ref mut unix) => unix.try_write_vectored(bufs),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }

    #[inline]
    fn poll_read_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match *self {
            Self::Tcp(ref mut tcp) => tcp.poll_read_ready(cx),
            #[cfg(unix)]
            Self::Unix(ref mut unix) => unix.poll_read_ready(cx),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }

    #[inline]
    fn poll_write_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match *self {
            Self::Tcp(ref mut tcp) => tcp.poll_write_ready(cx),
            #[cfg(unix)]
            Self::Unix(ref mut unix) => unix.poll_write_ready(cx),
            #[cfg(feature = "http3")]
            Self::Udp(..) => unreachable!("UdpStream can not implement AsyncRead/Write traits"),
        }
    }
}
