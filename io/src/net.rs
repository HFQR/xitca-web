//! re-export of [tokio::net] types

#[cfg(feature = "runtime-uring")]
pub mod io_uring;

#[cfg(feature = "quic")]
mod quic;
mod tcp;
#[cfg(unix)]
mod unix;

#[cfg(feature = "quic")]
pub use quic::*;
#[cfg(not(target_family = "wasm"))]
pub use tcp::TcpSocket;
pub use tcp::{TcpListener, TcpStream};
#[cfg(unix)]
pub use unix::{UnixListener, UnixStream};

use core::net::SocketAddr;

macro_rules! default_aio_impl {
    ($ty: ty) => {
        impl crate::io::AsyncIo for $ty {
            #[inline]
            async fn ready(&mut self, interest: crate::io::Interest) -> ::std::io::Result<crate::io::Ready> {
                self.0.ready(interest).await
            }

            fn poll_ready(
                &mut self,
                interest: crate::io::Interest,
                cx: &mut ::core::task::Context<'_>,
            ) -> ::core::task::Poll<::std::io::Result<crate::io::Ready>> {
                match interest {
                    crate::io::Interest::READABLE => self.0.poll_read_ready(cx).map_ok(|_| crate::io::Ready::READABLE),
                    crate::io::Interest::WRITABLE => self.0.poll_write_ready(cx).map_ok(|_| crate::io::Ready::WRITABLE),
                    _ => unimplemented!("tokio does not support poll_ready for BOTH read and write ready"),
                }
            }

            fn is_vectored_write(&self) -> bool {
                crate::io::AsyncWrite::is_write_vectored(&self.0)
            }

            fn poll_shutdown(
                self: ::core::pin::Pin<&mut Self>,
                cx: &mut ::core::task::Context<'_>,
            ) -> ::core::task::Poll<::std::io::Result<()>> {
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

/// A collection of stream types of different protocol.
#[allow(clippy::large_enum_variant)]
pub enum Stream {
    Tcp(std::net::TcpStream, SocketAddr),
    #[cfg(feature = "quic")]
    Udp(QuicStream, SocketAddr),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream, std::os::unix::net::SocketAddr),
}
