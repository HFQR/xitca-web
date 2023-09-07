use futures_core::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{error::Error, http::Version};

use super::stream::{Io, TlsStream};

#[cfg(feature = "openssl")]
use {
    openssl_crate::ssl::{SslConnector, SslMethod},
    tokio_openssl::SslStream,
};

#[cfg(feature = "rustls")]
use {
    std::sync::Arc,
    tokio_rustls::{
        rustls::{client::ServerName, ClientConfig, OwnedTrustAnchor, RootCertStore},
        TlsConnector,
    },
    webpki_roots::TLS_SERVER_ROOTS,
};

/// Connector for tls connections.
///
/// All connections are passed to tls connector. Non tls connections would be returned
/// with a noop pass through.
pub enum Connector {
    NoOp,
    #[cfg(feature = "openssl")]
    Openssl(SslConnector),
    #[cfg(feature = "rustls")]
    Rustls(TlsConnector),
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
        use xitca_http::bytes::BufMut;
        let mut alpn = Vec::with_capacity(20);
        for proto in protocols {
            alpn.put_u8(proto.len() as u8);
            alpn.put(*proto);
        }

        let mut ssl = SslConnector::builder(SslMethod::tls()).unwrap();

        ssl.set_alpn_protos(&alpn)
            .unwrap_or_else(|e| panic!("Can not set ALPN protocol: {e:?}"));

        Self::Openssl(ssl.build())
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(protocols: &[&[u8]]) -> Self {
        let mut root_certs = RootCertStore::empty();
        for cert in TLS_SERVER_ROOTS {
            let cert =
                OwnedTrustAnchor::from_subject_spki_name_constraints(cert.subject, cert.spki, cert.name_constraints);
            let certs = vec![cert].into_iter();
            root_certs.add_trust_anchors(certs);
        }

        let mut config = ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_certs)
            .with_no_client_auth();

        config.alpn_protocols = protocols.iter().map(|p| p.to_vec()).collect();

        Self::Rustls(TlsConnector::from(Arc::new(config)))
    }

    pub(crate) fn custom(connector: impl TlsConnect + 'static) -> Self {
        Self::Custom(Box::new(connector))
    }

    #[allow(unused_variables)]
    pub(crate) async fn connect<S>(&self, stream: S, domain: &str) -> Result<(TlsStream, Version), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        match *self {
            Self::NoOp => {
                #[cfg(not(feature = "dangerous"))]
                {
                    Err(Error::TlsNotEnabled)
                }

                #[cfg(feature = "dangerous")]
                {
                    // Enable HTTP/2 over plain TCP connection with dangerous feature.
                    //
                    // *. This is meant for test and local network usage. DO NOT use in internet environment.
                    Ok((Box::new(stream), Version::HTTP_2))
                }
            }
            Self::Custom(ref connector) => {
                let (stream, version) = connector.connect(Box::new(stream)).await?;
                Ok((stream, version))
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

                Ok((Box::new(stream), version))
            }
            #[cfg(feature = "rustls")]
            Self::Rustls(ref connector) => {
                let name = ServerName::try_from(domain).map_err(|_| crate::error::RustlsError::InvalidDnsName)?;
                let stream = connector
                    .connect(name, stream)
                    .await
                    .map_err(crate::error::RustlsError::Io)?;

                let version = stream.get_ref().1.alpn_protocol().map_or(Version::HTTP_11, |version| {
                    if version.windows(2).any(|w| w == b"h2") {
                        Version::HTTP_2
                    } else {
                        Version::HTTP_11
                    }
                });

                Ok((Box::new(stream), version))
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
pub trait TlsConnect: Send + Sync {
    /// `Box<dyn Io>` is an async read/write type. See [Io] trait for detail.
    #[allow(clippy::type_complexity)]
    fn connect<'s, 'f>(&'s self, io: Box<dyn Io>) -> BoxFuture<'f, Result<(Box<dyn Io>, Version), Error>>
    where
        's: 'f;
}
