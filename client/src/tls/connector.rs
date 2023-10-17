use core::future::Future;

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
    Nop,
    Custom(Box<dyn TlsConnectDyn>),
}

impl Default for Connector {
    fn default() -> Self {
        Self::Nop
    }
}

impl Connector {
    #[cfg(feature = "openssl")]
    pub(crate) fn openssl(protocols: &[&[u8]]) -> Self {
        use xitca_http::bytes::BufMut;
        let mut alpn = Vec::with_capacity(20);
        for proto in protocols {
            alpn.put_u8(proto.len() as u8);
            alpn.put(*proto);
        }

        let mut ssl = SslConnector::builder(SslMethod::tls()).unwrap();

        ssl.set_alpn_protos(&alpn)
            .unwrap_or_else(|e| panic!("Can not set ALPN protocol: {e:?}"));

        Self::custom(ssl.build())
    }

    #[cfg(feature = "rustls")]
    pub(crate) fn rustls(protocols: &[&[u8]]) -> Self {
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

        Self::custom(TlsConnector::from(Arc::new(config)))
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
            Self::Nop => {
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
            Self::Custom(ref connector) => connector.connect_dyn(domain, Box::new(stream)).await,
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
/// impl TlsConnect for MyConnector {
///     async fn connect(&self, domain: &str, io: Box<dyn Io>) -> Result<(Box<dyn Io>, Version), Error> {
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
    fn connect(&self, domain: &str, io: Box<dyn Io>) -> impl Future<Output = ConnectResult> + Send;
}

type ConnectResult = Result<(Box<dyn Io>, Version), Error>;

pub(crate) trait TlsConnectDyn: Send + Sync {
    fn connect_dyn<'s, 'd>(&'s self, domain: &'d str, io: Box<dyn Io>) -> BoxFuture<'d, ConnectResult>
    where
        's: 'd;
}

impl<T> TlsConnectDyn for T
where
    T: TlsConnect,
{
    #[inline]
    fn connect_dyn<'s, 'd>(&'s self, domain: &'d str, io: Box<dyn Io>) -> BoxFuture<'d, ConnectResult>
    where
        's: 'd,
    {
        Box::pin(self.connect(domain, io))
    }
}

#[cfg(feature = "openssl")]
impl TlsConnect for SslConnector {
    async fn connect(&self, domain: &str, io: Box<dyn Io>) -> ConnectResult {
        let ssl = self.configure()?.into_ssl(domain)?;
        let mut stream = SslStream::new(ssl, io)?;

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
}

#[cfg(feature = "rustls")]
impl TlsConnect for TlsConnector {
    async fn connect(&self, domain: &str, io: Box<dyn Io>) -> ConnectResult {
        let name = ServerName::try_from(domain).map_err(|_| crate::error::RustlsError::InvalidDnsName)?;
        let stream = self.connect(name, io).await.map_err(crate::error::RustlsError::Io)?;

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
