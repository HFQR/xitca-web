use core::future::Future;

use std::{
    io,
    net::{Shutdown, SocketAddr},
};

pub use tokio_uring::net::TcpStream;

#[cfg(unix)]
pub use tokio_uring::net::UnixStream;

use crate::io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf, IoBufMut};

use super::Stream;

impl From<Stream> for TcpStream {
    fn from(stream: Stream) -> Self {
        match stream {
            Stream::Tcp(tcp, _) => Self::from_std(tcp.into_std().unwrap()),
            _ => unreachable!("Can not be casted to TcpStream"),
        }
    }
}

impl From<Stream> for (TcpStream, SocketAddr) {
    fn from(stream: Stream) -> Self {
        match stream {
            Stream::Tcp(tcp, addr) => (TcpStream::from_std(tcp.into_std().unwrap()), addr),
            #[allow(unreachable_patterns)]
            _ => unreachable!("Can not be casted to TcpStream"),
        }
    }
}

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
    use std::net::{Ipv4Addr, SocketAddrV4};

    use super::*;

    impl From<Stream> for UnixStream {
        fn from(stream: Stream) -> Self {
            match stream {
                Stream::Unix(unix, _) => Self::from_std(unix.into_std().unwrap()),
                _ => unreachable!("Can not be casted to UnixStream"),
            }
        }
    }

    impl From<Stream> for (UnixStream, SocketAddr) {
        fn from(stream: Stream) -> Self {
            match stream {
                // UnixStream can not have a ip based socket address but to keep consistence with TcpStream/UdpStream types a unspecified address
                // is hand out as a placeholder.
                Stream::Unix(unix, _) => (
                    UnixStream::from_std(unix.into_std().unwrap()),
                    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
                ),
                _ => unreachable!("Can not be casted to UnixStream"),
            }
        }
    }

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
