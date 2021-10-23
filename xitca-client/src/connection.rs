use tokio::net::TcpStream;

#[cfg(unix)]
use tokio::net::UnixStream;

use crate::tls::stream::TlsStream;

#[non_exhaustive]
pub enum Connection {
    Tcp(TlsStream<TcpStream>),
    #[cfg(unix)]
    Unix(TlsStream<UnixStream>),
    H2(()),
}

impl From<TlsStream<TcpStream>> for Connection {
    fn from(tcp: TlsStream<TcpStream>) -> Self {
        Self::Tcp(tcp)
    }
}

#[cfg(unix)]
impl From<TlsStream<UnixStream>> for Connection {
    fn from(unix: TlsStream<UnixStream>) -> Self {
        Self::Unix(unix)
    }
}
