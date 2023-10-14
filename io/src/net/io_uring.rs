use std::{
    io,
    net::{Shutdown, SocketAddr},
};

pub use tokio_uring::net::TcpStream;

#[cfg(unix)]
pub use tokio_uring::net::UnixStream;

use crate::io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf, IoBufMut};

use super::Stream;

impl TryFrom<Stream> for TcpStream {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        <(TcpStream, SocketAddr)>::try_from(stream).map(|(tcp, _)| tcp)
    }
}

impl TryFrom<Stream> for (TcpStream, SocketAddr) {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        match stream {
            Stream::Tcp(tcp, addr) => Ok((TcpStream::from_std(tcp), addr)),
            #[allow(unreachable_patterns)]
            _ => unreachable!("Can not be casted to TcpStream"),
        }
    }
}

impl AsyncBufRead for TcpStream {
    #[inline(always)]
    async fn read<B>(&self, buf: B) -> (io::Result<usize>, B)
    where
        B: IoBufMut,
    {
        TcpStream::read(self, buf).await
    }
}

impl AsyncBufWrite for TcpStream {
    #[inline(always)]
    async fn write<B>(&self, buf: B) -> (io::Result<usize>, B)
    where
        B: IoBuf,
    {
        TcpStream::write(self, buf).await
    }

    #[inline(always)]
    fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
        TcpStream::shutdown(self, direction)
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::net::SocketAddr;

    use super::*;

    impl TryFrom<Stream> for UnixStream {
        type Error = io::Error;

        fn try_from(stream: Stream) -> Result<Self, Self::Error> {
            <(UnixStream, SocketAddr)>::try_from(stream).map(|(tcp, _)| tcp)
        }
    }

    impl TryFrom<Stream> for (UnixStream, SocketAddr) {
        type Error = io::Error;

        fn try_from(stream: Stream) -> Result<Self, Self::Error> {
            match stream {
                Stream::Unix(unix, addr) => Ok((UnixStream::from_std(unix), addr)),
                #[allow(unreachable_patterns)]
                _ => unreachable!("Can not be casted to UnixStream"),
            }
        }
    }

    impl AsyncBufRead for UnixStream {
        #[inline(always)]
        async fn read<B>(&self, buf: B) -> (io::Result<usize>, B)
        where
            B: IoBufMut,
        {
            UnixStream::read(self, buf).await
        }
    }

    impl AsyncBufWrite for UnixStream {
        #[inline(always)]
        async fn write<B>(&self, buf: B) -> (io::Result<usize>, B)
        where
            B: IoBuf,
        {
            UnixStream::write(self, buf).await
        }

        #[inline(always)]
        fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
            UnixStream::shutdown(self, direction)
        }
    }
}
