use futures_core::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{error::Error, http::Version};

use super::stream::{Io, TlsStream};

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
    Custom(Box<dyn TlsConnect>),
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

    pub(crate) fn custom(connector: impl TlsConnect + 'static) -> Self {
        Self::Custom(Box::new(connector))
    }

    #[allow(unused_variables)]
    pub(crate) async fn connect<S>(&self, stream: S, domain: &str) -> Result<(TlsStream<S>, Version), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        match *self {
            Self::NoOp => Err(Error::TlsNotEnabled),
            Self::Custom(ref connector) => {
                let (stream, version) = connector.connect(Box::new(stream)).await?;
                Ok((stream.into(), version))
            }
            #[cfg(feature = "openssl")]
            Self::Openssl(ref connector) => {
                let ssl = connector.configure()?.into_ssl(domain)?;
                let mut stream = SslStream::new(ssl, stream)?;
                std::pin::Pin::new(&mut stream).connect().await?;

                let version = stream
                    .ssl()
                    .selected_alpn_protocol()
                    .map_or(Version::HTTP_11, |version| {
                        if version.windows(2).any(|w| w == b"h2") {
                            Version::HTTP_2
                        } else {
                            Version::HTTP_11
                        }
                    });

                Ok((stream.into(), version))
            }
        }
    }
}

/// Trait for custom tls connector.
///
/// # Examples
/// ```rust
/// use xitca_client::{error::Error, http::Version, ClientBuilder, Io, TlsConnect};
///
/// struct MyConnector;
///
/// #[async_trait::async_trait]
/// impl TlsConnect for MyConnector {
///     async fn connect(&self, io: Box<dyn Io>) -> Result<(Box<dyn Io>, Version), Error> {
///         // tls handshake logic
///         todo!()
///     }
/// }
///
/// # fn resolve() {
/// let client = ClientBuilder::new().tls_connector(MyConnector).finish();
/// # }
/// ```
pub trait TlsConnect: Send {
    /// Box<dyn Io> is an async read/write type.
    ///
    /// See [Io] trait for detail.
    fn connect<'s, 'f>(&'s self, io: Box<dyn Io>) -> BoxFuture<'f, Result<(Box<dyn Io>, Version), Error>>
    where
        's: 'f;
}
