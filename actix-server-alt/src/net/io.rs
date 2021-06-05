use std::{
    future::Future,
    io,
    task::{Context, Poll},
};

use bytes::BufMut;
use tokio::io::{AsyncRead, AsyncWrite, Interest, Ready};

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

            fn ready(&mut self, interest: Interest) -> Self::ReadyFuture<'_> {
                Self::ready(self, interest)
            }

            fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<usize> {
                Self::try_read_buf(self, buf)
            }

            fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
                Self::try_write(self, buf)
            }

            fn try_write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
                Self::try_write_vectored(self, bufs)
            }

            fn poll_read_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Self::poll_read_ready(self, cx)
            }

            fn poll_write_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Self::poll_write_ready(self, cx)
            }
        }
    };
}

basic_impl!(super::TcpStream);

#[cfg(unix)]
basic_impl!(super::UnixStream);
