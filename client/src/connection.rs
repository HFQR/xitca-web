use core::hash::Hash;

use std::io;

use xitca_io::io::{AsyncIoDyn, Interest};

use super::{
    http::uri::{Authority, PathAndQuery},
    pool::Ready,
    tls::TlsStream,
    uri::Uri,
};

impl<'a> Ready for Box<dyn AsyncIoDyn + Send + Sync + 'a> {
    // an exclusive connect is considered ready if it's ready to be written(not closed)
    // and have no left over bytes inside
    async fn ready(&mut self) -> Result<(), ()> {
        AsyncIoDyn::ready(self.as_mut(), Interest::WRITABLE)
            .await
            .map_err(|_| ())?;
        match io::Read::read(self.as_mut(), &mut [0; 1]) {
            Ok(_) => Err(()),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(_) => Err(()),
        }
    }
}

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

impl Ready for ConnectionShared {
    /// wait until the connection is ready to open a new stream, returning `Err`
    /// when the underlying connection is no longer usable — e.g. an h2 server
    /// has sent GOAWAY or the transport has terminated. on a healthy connection
    /// at max concurrent streams this awaits a free slot rather than reporting
    /// the connection as unusable. existing in-flight streams are unaffected.
    async fn ready(&mut self) -> Result<(), ()> {
        match self {
            #[cfg(feature = "http2")]
            Self::H2(c) => core::future::poll_fn(|cx| c.poll_ready(cx)).await.map_err(|_| ()),
            #[cfg(feature = "http3")]
            Self::H3(_) => {
                // TODO: h3 0.0.8 exposes no public readiness API on SendRequest.
                // The relevant ConnectionState trait (is_closing / get_conn_error)
                // lives behind h3's `i-implement-a-third-party-backend-and-opt-into-
                // breaking-changes` feature, which is an explicit upstream signal
                // that the API will break without semver. Revisit once h3 ships a
                // stable poll_ready on SendRequest.
                Ok(())
            }
            _ => unreachable!(),
        }
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
    Regular { authority: Authority, is_tls: bool },
    Unix { authority: Authority, path: PathAndQuery },
}

impl From<&Uri<'_>> for ConnectionKey {
    fn from(uri: &Uri<'_>) -> Self {
        match *uri {
            Uri::Tcp(uri) => ConnectionKey::Regular {
                authority: uri.authority().unwrap().clone(),
                is_tls: false,
            },
            Uri::Tls(uri) => ConnectionKey::Regular {
                authority: uri.authority().unwrap().clone(),
                is_tls: true,
            },
            Uri::Unix(uri) => ConnectionKey::Unix {
                authority: uri.authority().unwrap().clone(),
                path: uri.path_and_query().unwrap().clone(),
            },
        }
    }
}
