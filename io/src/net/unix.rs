use std::{
    io,
    os::unix::{
        io::{AsFd, BorrowedFd},
        net::{self, SocketAddr},
    },
    path::Path,
};

use super::Stream;

pub use tokio::net::UnixListener;

pub struct UnixStream(pub(crate) tokio::net::UnixStream);

impl UnixStream {
    pub async fn connect<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        tokio::net::UnixStream::connect(path).await.map(Self)
    }

    pub fn from_std(stream: net::UnixStream) -> io::Result<Self> {
        tokio::net::UnixStream::from_std(stream).map(Self)
    }

    pub fn into_std(self) -> io::Result<net::UnixStream> {
        self.0.into_std()
    }
}

impl TryFrom<Stream> for UnixStream {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        <(UnixStream, SocketAddr)>::try_from(stream).map(|(unix, _)| unix)
    }
}

impl TryFrom<Stream> for (UnixStream, SocketAddr) {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        match stream {
            Stream::Unix(unix, addr) => UnixStream::from_std(unix).map(|unix| (unix, addr)),
            _ => unreachable!("Can not be casted to UnixStream"),
        }
    }
}

impl AsFd for UnixStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

super::default_aio_impl!(UnixStream);
