use core::future::Future;

use std::{io, net::Shutdown};

pub use tokio_uring::net::TcpStream;

#[cfg(unix)]
pub use tokio_uring::net::UnixStream;

use crate::io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf, IoBufMut};

impl AsyncBufRead for TcpStream {
    type Future<'f, B> = impl Future<Output = (io::Result<usize>, B)> + 'f
        where
            Self: 'f,
            B: IoBufMut + 'f;

    #[inline(always)]
    fn read<B>(&self, buf: B) -> Self::Future<'_, B>
    where
        B: IoBufMut,
    {
        TcpStream::read(self, buf)
    }
}

impl AsyncBufWrite for TcpStream {
    type Future<'f, B> = impl Future<Output = (io::Result<usize>, B)> + 'f
        where
            Self: 'f,
            B: IoBuf + 'f;

    #[inline(always)]
    fn write<B>(&self, buf: B) -> Self::Future<'_, B>
    where
        B: IoBuf,
    {
        TcpStream::write(self, buf)
    }

    #[inline(always)]
    fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
        TcpStream::shutdown(self, direction)
    }
}

#[cfg(unix)]
mod unix {
    use super::*;

    impl AsyncBufRead for UnixStream {
        type Future<'f, B> = impl Future<Output = (io::Result<usize>, B)> + 'f
            where
                Self: 'f,
                B: IoBufMut+ 'f;

        #[inline(always)]
        fn read<B>(&self, buf: B) -> Self::Future<'_, B>
        where
            B: IoBufMut,
        {
            UnixStream::read(self, buf)
        }
    }

    impl AsyncBufWrite for UnixStream {
        type Future<'f, B> = impl Future<Output = (io::Result<usize>, B)> + 'f
            where
                Self: 'f,
                B: IoBuf + 'f;

        #[inline(always)]
        fn write<B>(&self, buf: B) -> Self::Future<'_, B>
        where
            B: IoBuf,
        {
            UnixStream::write(self, buf)
        }

        #[inline(always)]
        fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
            UnixStream::shutdown(self, direction)
        }
    }
}
