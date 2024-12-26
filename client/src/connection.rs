use core::hash::{Hash, Hasher};

use xitca_http::http::uri::{Authority, PathAndQuery};

use super::{connect::Connect, request::SniHostname, tls::TlsStream, uri::Uri};

/// exclusive connection for http1 and in certain case they can be upgraded to [ConnectionShared]
pub type ConnectionExclusive = TlsStream;

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
    Regular(AuthorityWithSni),
    Unix(AuthorityWithPath),
}

#[doc(hidden)]
#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub struct AuthorityWithSni {
    authority: Authority,
    sni: Option<SniHostname>,
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

impl From<&Connect<'_>> for ConnectionKey {
    fn from(connect: &Connect<'_>) -> Self {
        match connect.uri {
            Uri::Tcp(uri) | Uri::Tls(uri) => ConnectionKey::Regular(AuthorityWithSni {
                authority: uri.authority().unwrap().clone(),
                sni: connect.sni_hostname.cloned(),
            }),
            Uri::Unix(uri) => ConnectionKey::Unix(AuthorityWithPath {
                authority: uri.authority().unwrap().clone(),
                path_and_query: uri.path_and_query().unwrap().clone(),
            }),
        }
    }
}
