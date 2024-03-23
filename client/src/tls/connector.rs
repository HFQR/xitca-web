use crate::{
    error::Error,
    http::Version,
    service::{Service, ServiceDyn},
};

use super::stream::Io;

/// Connector for tls connections.
///
/// All connections are passed to tls connector. Non tls connections would be returned
/// with a noop pass through.
pub type Connector =
    Box<dyn for<'n> ServiceDyn<(&'n str, Box<dyn Io>), Response = (Box<dyn Io>, Version), Error = Error> + Send + Sync>;

pub(crate) fn nop() -> Connector {
    struct Nop;

    impl<'n> Service<(&'n str, Box<dyn Io>)> for Nop {
        type Response = (Box<dyn Io>, Version);
        type Error = Error;

        async fn call(&self, (_, _io): (&'n str, Box<dyn Io>)) -> Result<Self::Response, Self::Error> {
            #[cfg(not(feature = "dangerous"))]
            {
                Err(Error::TlsNotEnabled)
            }

            #[cfg(feature = "dangerous")]
            {
                // Enable HTTP/2 over plain TCP connection with dangerous feature.
                //
                // *. This is meant for test and local network usage. DO NOT use in internet environment.
                Ok((_io, Version::HTTP_2))
            }
        }
    }

    Box::new(Nop)
}

#[cfg(feature = "openssl")]
pub(crate) mod openssl {
    use core::pin::Pin;

    use openssl_crate::ssl::{SslConnector, SslMethod};
    use tokio_openssl::SslStream;
    use xitca_http::bytes::BufMut;

    use super::*;

    impl<'n> Service<(&'n str, Box<dyn Io>)> for SslConnector {
        type Response = (Box<dyn Io>, Version);
        type Error = Error;

        async fn call(&self, (name, io): (&'n str, Box<dyn Io>)) -> Result<Self::Response, Self::Error> {
            let ssl = self.configure()?.into_ssl(name)?;
            let mut stream = SslStream::new(ssl, io)?;

            Pin::new(&mut stream).connect().await?;

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

    pub(crate) fn connect(protocols: &[&[u8]]) -> Connector {
        let mut alpn = Vec::with_capacity(20);
        for proto in protocols {
            alpn.put_u8(proto.len() as u8);
            alpn.put(*proto);
        }

        let mut ssl = SslConnector::builder(SslMethod::tls()).unwrap();

        ssl.set_alpn_protos(&alpn)
            .unwrap_or_else(|e| panic!("Can not set ALPN protocol: {e:?}"));

        Box::new(ssl.build())
    }
}

#[cfg(feature = "rustls")]
pub(crate) mod rustls {
    use std::sync::Arc;

    use tokio_rustls::{
        rustls::{pki_types::ServerName, ClientConfig, RootCertStore},
        TlsConnector,
    };
    use webpki_roots::TLS_SERVER_ROOTS;

    use super::*;

    impl<'n> Service<(&'n str, Box<dyn Io>)> for TlsConnector {
        type Response = (Box<dyn Io>, Version);
        type Error = Error;

        async fn call(&self, (name, io): (&'n str, Box<dyn Io>)) -> Result<Self::Response, Self::Error> {
            let name = ServerName::try_from(name)
                .map_err(|_| crate::error::RustlsError::InvalidDnsName)?
                .to_owned();
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

    pub(crate) fn connect(protocols: &[&[u8]]) -> Connector {
        let mut root_certs = RootCertStore::empty();

        root_certs.extend(TLS_SERVER_ROOTS.iter().cloned());

        let mut config = ClientConfig::builder()
            .with_root_certificates(root_certs)
            .with_no_client_auth();

        config.alpn_protocols = protocols.iter().map(|p| p.to_vec()).collect();

        Box::new(TlsConnector::from(Arc::new(config)))
    }
}
