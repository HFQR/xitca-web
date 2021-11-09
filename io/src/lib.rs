//! Async traits and types used for Io operations.

#![feature(generic_associated_types, type_alias_impl_trait)]

/// re-export of [bytes] crate types.
pub use bytes;

/// re-export of [tokio::net] types
pub mod net {
    pub use tokio::net::{TcpListener, TcpSocket, TcpStream};

    #[cfg(unix)]
    pub use tokio::net::{UnixListener, UnixStream};
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
