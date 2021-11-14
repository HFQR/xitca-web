//! Async traits and types used for Io operations.
#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

#[cfg(feature = "http3")]
mod h3;

/// re-export of [bytes] crate types.
pub use bytes;

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
}
