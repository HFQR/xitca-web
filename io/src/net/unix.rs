use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    os::unix::io::{AsRawFd, RawFd},
};

use super::Stream;

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
            Stream::Unix(unix, _) => unix,
            _ => unreachable!("Can not be casted to UnixStream"),
        }
    }
}

impl From<Stream> for (UnixStream, SocketAddr) {
    fn from(stream: Stream) -> Self {
        match stream {
            // UnixStream can not have a ip based socket address but to keep consistence with TcpStream/UdpStream types a unspecified address
            // is hand out as a placeholder.
            Stream::Unix(unix, _) => (unix, SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))),
            _ => unreachable!("Can not be casted to UnixStream"),
        }
    }
}

impl From<Stream> for (UnixStream, tokio::net::unix::SocketAddr) {
    fn from(stream: Stream) -> Self {
        match stream {
            Stream::Unix(unix, addr) => (unix, addr),
            _ => unreachable!("Can not be casted to UnixStream"),
        }
    }
}

impl AsRawFd for UnixStream {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

super::default_aio_impl!(UnixStream);
super::default_async_read_write_impl!(UnixStream);
