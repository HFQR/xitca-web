use std::{
    hash::{Hash, Hasher},
    io::{self, IoSlice},
    pin::Pin,
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

use xitca_http::http::uri::{Authority, PathAndQuery};

use crate::uri::Uri;

#[cfg(unix)]
use tokio::net::UnixStream;

use crate::tls::stream::TlsStream;

#[cfg(feature = "http1")]
/// A convince type alias for typing connection without interacting with pool.
pub type ConnectionWithKey<'a> = crate::pool::Conn<'a, ConnectionKey, Connection>;

/// Connection type branched into different HTTP version/layer.
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
pub enum Connection {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(feature = "http2")]
    H2(crate::h2::Connection),
    #[cfg(feature = "http3")]
    H3(crate::h3::Connection),
}

impl AsyncRead for Connection {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            Self::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!("http2 connection can not use AsyncRead"),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!("http3 connection can not use AsyncRead"),
        }
    }
}

impl AsyncWrite for Connection {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            Self::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!("http2 connection can not use AsyncWrite"),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!("http3 connection can not use AsyncRead"),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            Self::Tls(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!("http2 connection can not use AsyncWrite"),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!("http3 connection can not use AsyncRead"),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            Self::Tls(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!("http2 connection can not use AsyncWrite"),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!("http3 connection can not use AsyncRead"),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
            Self::Tls(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!("http2 connection can not use AsyncWrite"),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!("http3 connection can not use AsyncRead"),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::Tcp(stream) => stream.is_write_vectored(),
            Self::Tls(stream) => stream.is_write_vectored(),
            #[cfg(unix)]
            Self::Unix(stream) => stream.is_write_vectored(),
            #[cfg(feature = "http2")]
            Self::H2(_) => unimplemented!("http2 connection can not use AsyncWrite"),
            #[cfg(feature = "http3")]
            Self::H3(_) => unimplemented!("http3 connection can not use AsyncRead"),
        }
    }
}

impl From<TcpStream> for Connection {
    fn from(tcp: TcpStream) -> Self {
        Self::Tcp(tcp)
    }
}

impl From<TlsStream<TcpStream>> for Connection {
    fn from(tcp: TlsStream<TcpStream>) -> Self {
        Self::Tls(tcp)
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
