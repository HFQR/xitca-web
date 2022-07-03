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
    pub use tokio::net::{TcpListener, TcpSocket};

    use std::io;

    pub struct TcpStream(pub(crate) tokio::net::TcpStream);

    impl TcpStream {
        pub fn from_std(stream: std::net::TcpStream) -> io::Result<Self> {
            let stream = tokio::net::TcpStream::from_std(stream)?;
            Ok(Self(stream))
        }

        pub fn into_std(self) -> io::Result<std::net::TcpStream> {
            self.0.into_std()
        }
    }

    #[cfg(unix)]
    pub use unix::{UnixListener, UnixStream};

    #[cfg(unix)]
    mod unix {
        use std::{
            io,
            os::unix::io::{AsRawFd, RawFd},
        };

        use super::{Stream, TcpStream};

        pub use tokio::net::UnixListener;

        pub struct UnixStream(pub(crate) tokio::net::UnixStream);

        impl UnixStream {
            pub fn from_std(stream: std::os::unix::net::UnixStream) -> io::Result<Self> {
                let stream = tokio::net::UnixStream::from_std(stream)?;
                Ok(Self(stream))
            }

            pub fn into_std(self) -> io::Result<std::os::unix::net::UnixStream> {
                self.0.into_std()
            }
        }

        impl From<Stream> for UnixStream {
            fn from(stream: Stream) -> Self {
                match stream {
                    Stream::Unix(unix) => unix,
                    _ => unreachable!("Can not be casted to UnixStream"),
                }
            }
        }

        impl AsRawFd for UnixStream {
            fn as_raw_fd(&self) -> RawFd {
                self.0.as_raw_fd()
            }
        }

        impl AsRawFd for TcpStream {
            fn as_raw_fd(&self) -> RawFd {
                self.0.as_raw_fd()
            }
        }
    }

    #[cfg(windows)]
    mod windows {
        use std::{
            io,
            os::windows::io::{AsRawSocket, RawSocket},
        };

        use super::TcpStream;

        impl AsRawSocket for TcpStream {
            fn as_raw_socket(&self) -> RawSocket {
                self.0.as_raw_socket
            }
        }
    }

    #[cfg(feature = "http3")]
    pub use super::h3::*;

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

    use core::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use std::io;

    /// A wrapper trait for an AsyncRead/AsyncWrite tokio type with additional methods.
    pub trait AsyncIo: io::Read + io::Write + Unpin {
        type ReadyFuture<'f>: Future<Output = io::Result<Ready>>
        where
            Self: 'f;

        /// asynchronously wait for the IO type and return it's state as [Ready].
        fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_>;

        /// hint if IO can be vectored write.
        fn is_vectored_write(&self) -> bool;

        /// poll shutdown the write part of Self.
        ///
        /// # Why:
        /// tokio's network Stream types does not expose other api for shutdown besides
        /// [AsyncWrite::poll_shutdown].
        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
    }

    macro_rules! basic_impl {
        ($ty: ty) => {
            impl AsyncIo for $ty {
                type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>>;

                #[inline]
                fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_> {
                    self.0.ready(interest)
                }

                #[inline]
                fn is_vectored_write(&self) -> bool {
                    self.0.is_write_vectored()
                }

                #[inline]
                fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                    AsyncWrite::poll_shutdown(Pin::new(&mut self.get_mut().0), cx)
                }
            }

            impl io::Read for $ty {
                #[inline]
                fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                    self.0.try_read(buf)
                }
            }

            impl io::Write for $ty {
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
        };
    }

    basic_impl!(super::net::TcpStream);

    #[cfg(unix)]
    basic_impl!(super::net::UnixStream);
}
