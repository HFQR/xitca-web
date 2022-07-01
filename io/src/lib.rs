//! Async traits and types used for Io operations.

#![feature(generic_associated_types, type_alias_impl_trait)]

#[cfg(feature = "http3")]
mod h3;

/// re-export of [bytes] crate types.
pub mod bytes {
    pub use bytes::*;

    use std::{fmt, io};

    /// A new type for help implementing [std::io::Write] and [std::fmt::Write] traits.
    pub struct BufMutWriter<'a, B>(pub &'a mut B);

    impl<B: BufMut> io::Write for BufMutWriter<'_, B> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.put_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<B: BufMut> fmt::Write for BufMutWriter<'_, B> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            self.0.put_slice(s.as_bytes());
            Ok(())
        }
    }
}

/// re-export of [tokio::net] types
pub mod net {
    pub use tokio::net::{TcpListener, TcpSocket, TcpStream};

    #[cfg(unix)]
    pub use tokio::net::{UnixListener, UnixStream};

    #[cfg(feature = "http3")]
    pub use crate::h3::*;

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
        pub async fn accept(&self) -> std::io::Result<Stream> {
            match *self {
                Self::Tcp(ref tcp) => {
                    let (stream, _) = tcp.accept().await?;

                    // This two way conversion is to deregister stream from the listener thread's poll
                    // and re-register it to current thread's poll.
                    let stream = stream.into_std()?;
                    let stream = TcpStream::from_std(stream)?;
                    Ok(Stream::Tcp(stream))
                }
                #[cfg(feature = "http3")]
                Self::Udp(ref udp) => {
                    let stream = udp.accept().await?;
                    Ok(Stream::Udp(stream))
                }
                #[cfg(unix)]
                Self::Unix(ref unix) => {
                    let (stream, _) = unix.accept().await?;

                    // This two way conversion is to deregister stream from the listener thread's poll
                    // and re-register it to current thread's poll.
                    let stream = stream.into_std()?;
                    let stream = UnixStream::from_std(stream)?;
                    Ok(Stream::Unix(stream))
                }
            }
        }
    }

    /// A collection of stream types of different protocol.
    pub enum Stream {
        Tcp(TcpStream),
        #[cfg(feature = "http3")]
        Udp(UdpStream),
        #[cfg(unix)]
        Unix(UnixStream),
    }

    impl From<Stream> for TcpStream {
        fn from(stream: Stream) -> Self {
            match stream {
                Stream::Tcp(tcp) => tcp,
                #[cfg(any(not(windows), feature = "http3"))]
                _ => unreachable!("Can not be casted to TcpStream"),
            }
        }
    }

    #[cfg(unix)]
    impl From<Stream> for UnixStream {
        fn from(stream: Stream) -> Self {
            match stream {
                Stream::Unix(unix) => unix,
                _ => unreachable!("Can not be casted to UnixStream"),
            }
        }
    }

    #[cfg(feature = "http3")]
    impl From<Stream> for UdpStream {
        fn from(stream: Stream) -> Self {
            match stream {
                Stream::Udp(udp) => udp,
                _ => unreachable!("Can not be casted to UdpStream"),
            }
        }
    }
}

/// re-export of [tokio::io] types and extended AsyncIo trait on top of it.
pub mod io {
    pub use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};

    use std::{future::Future, io};

    use crate::bytes::BufMut;

    /// A wrapper trait for an AsyncRead/AsyncWrite tokio type with additional methods.
    pub trait AsyncIo: AsyncRead + AsyncWrite + Unpin {
        type ReadyFuture<'f>: Future<Output = io::Result<Ready>>
        where
            Self: 'f;

        /// asynchronously wait for the IO type and
        fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_>;

        fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize>;

        fn try_write(&mut self, buf: &[u8]) -> io::Result<usize>;

        fn try_write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize>;
    }

    macro_rules! basic_impl {
        ($ty: ty) => {
            impl AsyncIo for $ty {
                type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>>;

                #[inline]
                fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_> {
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
            }
        };
    }

    basic_impl!(tokio::net::TcpStream);

    #[cfg(unix)]
    basic_impl!(tokio::net::UnixStream);

    /// An adapter for [AsyncIo] type to implement [io::Writer] trait for it.
    pub struct StdIoAdapter<'a, Io>(pub &'a mut Io);

    impl<Io> io::Write for StdIoAdapter<'_, Io>
    where
        Io: AsyncIo,
    {
        #[inline]
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.try_write(buf)
        }

        #[inline]
        fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
            self.0.try_write_vectored(bufs)
        }

        #[inline]
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
