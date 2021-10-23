use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::Error;

use super::stream::TlsStream;

#[cfg(feature = "openssl")]
use {openssl_crate::ssl::SslConnector, tokio_openssl::SslStream};

/// Connector for tls connections.
///
/// All connections are passed to tls connector. Non tls connections would be returned
/// with a noop pass through.
pub enum Connector {
    NoOp,
    #[cfg(feature = "openssl")]
    Openssl(SslConnector),
}

impl Default for Connector {
    fn default() -> Self {
        Self::NoOp
    }
}

#[cfg(feature = "openssl")]
impl From<SslConnector> for Connector {
    fn from(connector: SslConnector) -> Self {
        Self::Openssl(connector)
    }
}

impl Connector {
    #[cfg(feature = "openssl")]
    pub fn openssl(protocols: &[&[u8]]) -> Self {
        todo!()
    }

    pub(crate) async fn connect<S>(&self, stream: S, domain: &str) -> Result<TlsStream<S>, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        match *self {
            Self::NoOp => {
                drop(domain);
                Ok(TlsStream::NoOp(stream))
            },
            #[cfg(feature = "openssl")]
            Self::Openssl(ref connector) => {
                let ssl = connector.configure()?.into_ssl(domain)?;
                let mut stream = SslStream::new(ssl, stream)?;
                std::pin::Pin::new(&mut stream).connect().await?;
                Ok(TlsStream::Openssl(stream))
            }
        }
    }
}
