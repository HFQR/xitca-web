use std::{io, net::SocketAddr};

use super::Stream;

// TODO: possible remove the attribute when wasm support tcp socket.
#[cfg(not(target_family = "wasm"))]
pub use tokio::net::TcpSocket;

pub struct TcpStream(pub(crate) tokio::net::TcpStream);

impl TcpStream {
    // TODO: possible remove the attribute when wasm support tcp connect.
    #[cfg(not(target_family = "wasm"))]
    pub async fn connect<A: tokio::net::ToSocketAddrs>(addr: A) -> io::Result<Self> {
        tokio::net::TcpStream::connect(addr).await.map(Self)
    }

    pub fn from_std(stream: std::net::TcpStream) -> io::Result<Self> {
        let stream = tokio::net::TcpStream::from_std(stream)?;
        Ok(Self(stream))
    }

    pub fn into_std(self) -> io::Result<std::net::TcpStream> {
        self.0.into_std()
    }

    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.0.set_nodelay(nodelay)
    }
}

impl From<Stream> for TcpStream {
    fn from(stream: Stream) -> Self {
        match stream {
            Stream::Tcp(tcp, _) => tcp,
            #[allow(unreachable_patterns)]
            _ => unreachable!("Can not be casted to TcpStream"),
        }
    }
}

impl From<Stream> for (TcpStream, SocketAddr) {
    fn from(stream: Stream) -> Self {
        match stream {
            Stream::Tcp(tcp, addr) => (tcp, addr),
            #[allow(unreachable_patterns)]
            _ => unreachable!("Can not be casted to TcpStream"),
        }
    }
}

super::default_async_read_write_impl!(TcpStream);

super::default_aio_impl!(TcpStream);

#[cfg(unix)]
mod unix_impl {
    use std::os::unix::io::{AsRawFd, RawFd};

    use super::TcpStream;

    impl AsRawFd for TcpStream {
        fn as_raw_fd(&self) -> RawFd {
            self.0.as_raw_fd()
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::os::windows::io::{AsRawSocket, RawSocket};

    use super::TcpStream;

    impl AsRawSocket for TcpStream {
        fn as_raw_socket(&self) -> RawSocket {
            self.0.as_raw_socket()
        }
    }
}
