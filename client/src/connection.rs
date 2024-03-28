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
pub type ConnectionWithKey<'a> = crate::pool::Conn<'a, ConnectionKey, Connection>;

#[cfg(feature = "http1")]
/// A convince type alias for typing connection without interacting with pool.
pub type ConnectionWithoutKey = crate::pool::PooledConn<Connection>;

/// Connection type branched into different HTTP version/layer.
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
pub enum Connection {
    Tcp(TcpStream),
    Tls(TlsStream),
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(feature = "http2")]
    H2(crate::h2::Connection),
    #[cfg(feature = "http3")]
    H3(crate::h3::Connection),
}

impl AsyncIo for Connection {
    fn ready(&self, interest: Interest) -> impl std::future::Future<Output = io::Result<Ready>> + Send {
        // AsyncIo::ready must be producing Send future and with reference as &Connection as self
        // the Sync trait bound is needed. h2 and h3 Connections are not Sync therefore a type conversion
        // is necessary.
        // TODO: consider make the type public with background conversion when it's popped from/pushed back
        // to connection pool.
        enum ConnectionSync<'a> {
            Tcp(&'a TcpStream),
            Tls(&'a TlsStream),
            #[cfg(unix)]
            Unix(&'a UnixStream),
        }

        let conn = match self {
            Self::Tcp(ref io) => ConnectionSync::Tcp(io),
            Self::Tls(ref io) => ConnectionSync::Tls(io),
            #[cfg(unix)]
            Self::Unix(ref io) => ConnectionSync::Unix(io),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!(),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!(),
        };

        async move {
            match conn {
                ConnectionSync::Tcp(io) => io.ready(interest).await,
                ConnectionSync::Tls(io) => io.ready(interest).await,
                #[cfg(unix)]
                ConnectionSync::Unix(io) => io.ready(interest).await,
            }
        }
    }

    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        match self {
            Self::Tcp(ref io) => io.poll_ready(interest, cx),
            Self::Tls(ref io) => io.poll_ready(interest, cx),
            #[cfg(unix)]
            Self::Unix(ref io) => io.poll_ready(interest, cx),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!(),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!(),
        }
    }

    fn is_vectored_write(&self) -> bool {
        match self {
            Self::Tcp(ref io) => io.is_vectored_write(),
            Self::Tls(ref io) => io.is_vectored_write(),
            #[cfg(unix)]
            Self::Unix(ref io) => io.is_vectored_write(),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!(),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!(),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(io) => Pin::new(io).poll_shutdown(cx),
            Self::Tls(io) => Pin::new(io).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(io) => Pin::new(io).poll_shutdown(cx),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!(),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!(),
        }
    }
}

impl io::Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(ref mut io) => io.read(buf),
            Self::Tls(ref mut io) => io.read(buf),
            #[cfg(unix)]
            Self::Unix(ref mut io) => io.read(buf),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!(),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!(),
        }
    }
}

impl io::Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(ref mut io) => io.write(buf),
            Self::Tls(ref mut io) => io.write(buf),
            #[cfg(unix)]
            Self::Unix(ref mut io) => io.write(buf),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!(),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!(),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(ref mut io) => io.flush(),
            Self::Tls(ref mut io) => io.flush(),
            #[cfg(unix)]
            Self::Unix(ref mut io) => io.flush(),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!(),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!(),
        }
    }
}

impl From<TcpStream> for Connection {
    fn from(tcp: TcpStream) -> Self {
        Self::Tcp(tcp)
    }
}

impl From<TlsStream> for Connection {
    fn from(io: TlsStream) -> Self {
        Self::Tls(io)
    }
}

#[cfg(unix)]
impl From<UnixStream> for Connection {
    fn from(unix: UnixStream) -> Self {
        Self::Unix(unix)
    }
}

#[cfg(feature = "http2")]
impl From<crate::h2::Connection> for Connection {
    fn from(connection: crate::h2::Connection) -> Self {
        Self::H2(connection)
    }
}

#[cfg(feature = "http3")]
impl From<crate::h3::Connection> for Connection {
    fn from(connection: crate::h3::Connection) -> Self {
        Self::H3(connection)
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

#[doc(hidden)]
/// Trait for multiplex connection.
/// HTTP2 and HTTP3 connections are supposed to be multiplexed on single TCP connection.
pub trait Multiplex {
    /// Get a ownership from mut reference.
    ///
    /// # Panics:
    /// When called on connection type that are not multiplexable.
    fn multiplex(&mut self) -> Self;

    /// Return true for connection that can be multiplexed.
    fn is_multiplexable(&self) -> bool;
}

impl Multiplex for Connection {
    fn multiplex(&mut self) -> Self {
        match *self {
            #[cfg(feature = "http2")]
            Self::H2(ref conn) => Self::H2(conn.clone()),
            _ => unreachable!("Connection is not multiplexable"),
        }
    }

    fn is_multiplexable(&self) -> bool {
        match *self {
            #[cfg(feature = "http2")]
            Self::H2(_) => true,
            _ => false,
        }
    }
}
