use core::{
    hash::{Hash, Hasher},
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

use xitca_http::http::uri::{Authority, PathAndQuery};
use xitca_io::{
    io::{AsyncIo, Interest, Ready},
    net::TcpStream,
};

#[cfg(unix)]
use xitca_io::net::UnixStream;

use crate::{tls::stream::TlsStream, uri::Uri};

#[cfg(feature = "http1")]
/// A convince type alias for typing connection without interacting with pool.
pub type H1ConnectionWithKey<'a> = crate::pool::exclusive::Conn<'a, ConnectionKey, ConnectionExclusive>;

#[cfg(feature = "http1")]
/// A convince type alias for typing connection without interacting with pool.
pub type H1ConnectionWithoutKey = crate::pool::exclusive::PooledConn<ConnectionExclusive>;

/// exclusive connection for http1 and in certain case they can be upgraded to [ConnectionShared]
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
pub enum ConnectionExclusive {
    Tcp(TcpStream),
    Tls(TlsStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl AsyncIo for ConnectionExclusive {
    async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        match self {
            Self::Tcp(ref io) => io.ready(interest).await,
            Self::Tls(ref io) => io.ready(interest).await,
            #[cfg(unix)]
            Self::Unix(ref io) => io.ready(interest).await,
        }
    }

    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        match self {
            Self::Tcp(ref io) => io.poll_ready(interest, cx),
            Self::Tls(ref io) => io.poll_ready(interest, cx),
            #[cfg(unix)]
            Self::Unix(ref io) => io.poll_ready(interest, cx),
        }
    }

    fn is_vectored_write(&self) -> bool {
        match self {
            Self::Tcp(ref io) => io.is_vectored_write(),
            Self::Tls(ref io) => io.is_vectored_write(),
            #[cfg(unix)]
            Self::Unix(ref io) => io.is_vectored_write(),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(io) => Pin::new(io).poll_shutdown(cx),
            Self::Tls(io) => Pin::new(io).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(io) => Pin::new(io).poll_shutdown(cx),
        }
    }
}

impl io::Read for ConnectionExclusive {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(ref mut io) => io.read(buf),
            Self::Tls(ref mut io) => io.read(buf),
            #[cfg(unix)]
            Self::Unix(ref mut io) => io.read(buf),
        }
    }
}

impl io::Write for ConnectionExclusive {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(ref mut io) => io.write(buf),
            Self::Tls(ref mut io) => io.write(buf),
            #[cfg(unix)]
            Self::Unix(ref mut io) => io.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(ref mut io) => io.flush(),
            Self::Tls(ref mut io) => io.flush(),
            #[cfg(unix)]
            Self::Unix(ref mut io) => io.flush(),
        }
    }
}

impl From<TcpStream> for ConnectionExclusive {
    fn from(tcp: TcpStream) -> Self {
        Self::Tcp(tcp)
    }
}

impl From<TlsStream> for ConnectionExclusive {
    fn from(io: TlsStream) -> Self {
        Self::Tls(io)
    }
}

#[cfg(unix)]
impl From<UnixStream> for ConnectionExclusive {
    fn from(unix: UnixStream) -> Self {
        Self::Unix(unix)
    }
}

/// high level shared connection that support multiplexing over single socket
/// used for http2 and http3
#[derive(Clone)]
pub enum ConnectionShared {
    #[cfg(feature = "http2")]
    H2(crate::h2::Connection),
    #[cfg(feature = "http3")]
    H3(crate::h3::Connection),
}

#[cfg(feature = "http2")]
impl From<crate::h2::Connection> for ConnectionShared {
    fn from(conn: crate::h2::Connection) -> Self {
        Self::H2(conn)
    }
}

#[cfg(feature = "http3")]
impl From<crate::h3::Connection> for ConnectionShared {
    fn from(conn: crate::h3::Connection) -> Self {
        Self::H3(conn)
    }
}

#[doc(hidden)]
#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub enum ConnectionKey {
    Regular(Authority),
    #[cfg(unix)]
    Unix(AuthorityWithPath),
}

#[doc(hidden)]
#[derive(Eq, Debug, Clone)]
pub struct AuthorityWithPath {
    authority: Authority,
    path_and_query: PathAndQuery,
}

impl PartialEq for AuthorityWithPath {
    fn eq(&self, other: &Self) -> bool {
        self.authority.eq(&other.authority) && self.path_and_query.eq(&other.path_and_query)
    }
}

impl Hash for AuthorityWithPath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.authority.hash(state);
        self.path_and_query.as_str().hash(state);
    }
}

impl From<&Uri<'_>> for ConnectionKey {
    fn from(uri: &Uri<'_>) -> Self {
        match *uri {
            Uri::Tcp(uri) | Uri::Tls(uri) => ConnectionKey::Regular(uri.authority().unwrap().clone()),
            #[cfg(unix)]
            Uri::Unix(uri) => ConnectionKey::Unix(AuthorityWithPath {
                authority: uri.authority().unwrap().clone(),
                path_and_query: uri.path_and_query().unwrap().clone(),
            }),
        }
    }
}
