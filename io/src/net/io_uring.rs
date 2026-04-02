use core::net::SocketAddr;

use std::io;

pub use tokio_uring_xitca::net::{TcpStream, UnixStream};

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

impl TryFrom<Stream> for UnixStream {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        <(UnixStream, std::os::unix::net::SocketAddr)>::try_from(stream).map(|(tcp, _)| tcp)
    }
}

impl TryFrom<Stream> for (UnixStream, std::os::unix::net::SocketAddr) {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        match stream {
            Stream::Unix(unix, addr) => Ok((UnixStream::from_std(unix), addr)),
            #[allow(unreachable_patterns)]
            _ => unreachable!("Can not be casted to UnixStream"),
        }
    }
}
