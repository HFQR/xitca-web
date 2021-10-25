use std::hash::{Hash, Hasher};

use http::uri::{Authority, PathAndQuery};
use tokio::net::TcpStream;

use crate::uri::Uri;

#[cfg(unix)]
use tokio::net::UnixStream;

use crate::tls::stream::TlsStream;

#[non_exhaustive]
pub enum Connection {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(feature = "http2")]
    H2(()),
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
