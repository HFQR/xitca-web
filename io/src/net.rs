//! re-export of [tokio::net] types

#[cfg(feature = "runtime-uring")]
pub mod io_uring;

#[cfg(feature = "http3")]
mod h3;
mod tcp;
#[cfg(unix)]
mod unix;

#[cfg(feature = "http3")]
pub use h3::*;
#[cfg(not(target_family = "wasm"))]
pub use tcp::TcpSocket;
pub use tcp::{TcpListener, TcpStream};
#[cfg(unix)]
pub use unix::{UnixListener, UnixStream};

use std::{io, net::SocketAddr};

macro_rules! default_aio_impl {
    ($ty: ty) => {
        impl crate::io::AsyncIo for $ty {
            type Future<'f> = impl ::core::future::Future<Output = ::std::io::Result<crate::io::Ready>> + Send + 'f where Self: 'f;

            #[inline]
            fn ready(&self, interest: crate::io::Interest) -> Self::Future<'_> {
                self.0.ready(interest)
            }

            fn poll_ready(&self, interest: crate::io::Interest, cx: &mut ::core::task::Context<'_>) -> ::core::task::Poll<::std::io::Result<crate::io::Ready>> {
                match interest {
                    crate::io::Interest::READABLE => self.0.poll_read_ready(cx).map_ok(|_| crate::io::Ready::READABLE),
                    crate::io::Interest::WRITABLE => self.0.poll_write_ready(cx).map_ok(|_| crate::io::Ready::WRITABLE),
                    _ => unimplemented!("tokio does not support poll_ready for BOTH read and write ready"),
                }
            }

            fn is_vectored_write(&self) -> bool {
                 crate::io::AsyncWrite::is_write_vectored(&self.0)
            }

            fn poll_shutdown(self: ::core::pin::Pin<&mut Self>, cx: &mut ::core::task::Context<'_>) -> ::core::task::Poll<::std::io::Result<()>> {
                crate::io::AsyncWrite::poll_shutdown(::core::pin::Pin::new(&mut self.get_mut().0), cx)
            }
        }

        impl ::std::io::Read for $ty {
            #[inline]
            fn read(&mut self, buf: &mut [u8]) -> ::std::io::Result<usize> {
                self.0.try_read(buf)
            }
        }

        impl ::std::io::Write for $ty {
            #[inline]
            fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> {
                self.0.try_write(buf)
            }

            #[inline]
            fn write_vectored(&mut self, bufs: &[::std::io::IoSlice<'_>]) -> ::std::io::Result<usize> {
                self.0.try_write_vectored(bufs)
            }

            #[inline]
            fn flush(&mut self) -> ::std::io::Result<()> {
                Ok(())
            }
        }

        // specialized read implement based on tokio 1.0 spec.
        impl ::std::io::Read for &$ty {
            #[inline]
            fn read(&mut self, buf: &mut [u8]) -> ::std::io::Result<usize> {
                self.0.try_read(buf)
            }
        }

        // specialized implement based on tokio 1.0 spec.
        impl ::std::io::Write for &$ty {
            #[inline]
            fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> {
                self.0.try_write(buf)
            }

            #[inline]
            fn write_vectored(&mut self, bufs: &[::std::io::IoSlice<'_>]) -> ::std::io::Result<usize> {
                self.0.try_write_vectored(bufs)
            }

            #[inline]
            fn flush(&mut self) -> ::std::io::Result<()> {
                Ok(())
            }
        }
    };
}

use default_aio_impl;

// TODO: remove this macro and implements when a homebrew h2 protocol is introduced.
// temporary macro that forward AsyncRead/AsyncWrite trait.
// This is only used for h2 crate.
macro_rules! default_async_read_write_impl {
    ($ty: ty) => {
        impl crate::io::AsyncRead for $ty {
            #[inline]
            fn poll_read(
                self: ::core::pin::Pin<&mut Self>,
                cx: &mut ::core::task::Context<'_>,
                buf: &mut crate::io::ReadBuf<'_>,
            ) -> ::core::task::Poll<::std::io::Result<()>> {
                ::core::pin::Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
            }
        }

        impl crate::io::AsyncWrite for $ty {
            #[inline]
            fn poll_write(
                self: ::core::pin::Pin<&mut Self>,
                cx: &mut ::core::task::Context<'_>,
                buf: &[u8],
            ) -> ::core::task::Poll<::std::io::Result<usize>> {
                ::core::pin::Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
            }

            #[inline]
            fn poll_flush(
                self: ::core::pin::Pin<&mut Self>,
                cx: &mut ::core::task::Context<'_>,
            ) -> ::core::task::Poll<::std::io::Result<()>> {
                ::core::pin::Pin::new(&mut self.get_mut().0).poll_flush(cx)
            }

            #[inline]
            fn poll_shutdown(
                self: ::core::pin::Pin<&mut Self>,
                cx: &mut ::core::task::Context<'_>,
            ) -> ::core::task::Poll<::std::io::Result<()>> {
                ::core::pin::Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
            }

            #[inline]
            fn poll_write_vectored(
                self: ::core::pin::Pin<&mut Self>,
                cx: &mut ::core::task::Context<'_>,
                bufs: &[::std::io::IoSlice<'_>],
            ) -> ::core::task::Poll<::std::io::Result<usize>> {
                ::core::pin::Pin::new(&mut self.get_mut().0).poll_write_vectored(cx, bufs)
            }

            fn is_write_vectored(&self) -> bool {
                self.0.is_write_vectored()
            }
        }
    };
}

use default_async_read_write_impl;

/// A collection of listener types of different protocol.
#[derive(Debug)]
pub enum Listener {
    Tcp(TcpListener),
    #[cfg(feature = "http3")]
    Udp(UdpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

impl Listener {
    pub async fn accept(&self) -> io::Result<Stream> {
        match *self {
            Self::Tcp(ref tcp) => {
                let (stream, addr) = tcp.accept().await?;

                // This two way conversion is to deregister stream from the listener thread's poll
                // and re-register it to current thread's poll.
                let stream = stream.into_std()?;
                let stream = TcpStream::from_std(stream)?;
                Ok(Stream::Tcp(stream, addr))
            }
            #[cfg(feature = "http3")]
            Self::Udp(ref udp) => {
                let stream = udp.accept().await?;
                let addr = stream.peer_addr();
                Ok(Stream::Udp(stream, addr))
            }
            #[cfg(unix)]
            Self::Unix(ref unix) => {
                let (stream, addr) = unix.accept().await?;

                // This two way conversion is to deregister stream from the listener thread's poll
                // and re-register it to current thread's poll.
                let stream = stream.into_std()?;
                let stream = UnixStream::from_std(stream)?;
                Ok(Stream::Unix(stream, addr))
            }
        }
    }
}

/// A collection of stream types of different protocol.
pub enum Stream {
    Tcp(TcpStream, SocketAddr),
    #[cfg(feature = "http3")]
    Udp(UdpStream, SocketAddr),
    #[cfg(unix)]
    Unix(UnixStream, tokio::net::unix::SocketAddr),
}
