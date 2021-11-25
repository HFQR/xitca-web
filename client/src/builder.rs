use std::{net::SocketAddr, time::Duration};

use xitca_http::http::version::Version;

use crate::{
    client::Client,
    date::DateTimeService,
    pool::Pool,
    resolver::{Resolve, Resolver},
    timeout::TimeoutConfig,
    tls::connector::{Connector, TlsConnect},
};

pub struct ClientBuilder {
    connector_builder: TlsConnectorBuilder,
    resolver: Resolver,
    pool_capacity: usize,
    timeout_config: TimeoutConfig,
    local_addr: Option<SocketAddr>,
    max_http_version: Version,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        ClientBuilder {
            connector_builder: TlsConnectorBuilder::Default,
            resolver: Resolver::default(),
            pool_capacity: 128,
            timeout_config: TimeoutConfig::default(),
            local_addr: None,
            max_http_version: Version::HTTP_3,
        }
    }
}

impl ClientBuilder {
    pub fn new() -> Self {
        ClientBuilder {
            #[cfg(all(feature = "openssl", not(feature = "rustls")))]
            connector_builder: TlsConnectorBuilder::Openssl,
            #[cfg(all(feature = "rustls", not(feature = "openssl")))]
            connector_builder: TlsConnectorBuilder::Rustls,
            ..Default::default()
        }
    }

    #[cfg(feature = "openssl")]
    pub fn openssl(mut self) -> Self {
        self.connector_builder = TlsConnectorBuilder::Openssl;
        self
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(mut self) -> Self {
        self.connector_builder = TlsConnectorBuilder::Rustls;
        self
    }

    /// Use custom DNS resolver for domain look up.
    ///
    /// See [Resolve] for detail.
    pub fn resolver(mut self, resolver: impl Resolve + 'static) -> Self {
        self.resolver = Resolver::custom(resolver);
        self
    }

    /// Use custom tls connector for tls handling.
    ///
    /// See [TlsConnect] for detail.
    pub fn tls_connector(mut self, connector: impl TlsConnect + 'static) -> Self {
        self.connector_builder = TlsConnectorBuilder::Custom(Connector::custom(connector));
        self
    }

    /// Set timeout for DNS resolve.
    ///
    /// Default to 5 seconds.
    pub fn set_resolve_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.resolve_timeout = dur;
        self
    }

    /// Set timeout for establishing connection.
    ///
    /// Default to 5 seconds.
    pub fn set_connect_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.connect_timeout = dur;
        self
    }

    /// Set timeout for tls handshake.
    ///
    /// Default to 5 seconds.
    pub fn set_tls_connect_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.tls_connect_timeout = dur;
        self
    }

    /// Set timeout for request.
    ///
    /// Default to 15 seconds.
    pub fn set_request_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.request_timeout = dur;
        self
    }

    /// Set timeout for collecting response body.
    ///
    /// Default to 15 seconds.
    pub fn set_response_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.response_timeout = dur;
        self
    }

    /// Set local Socket address, either IPv4 or IPv6 used for http client.
    ///
    /// By default client uses any free address the OS returns.
    pub fn set_local_addr(mut self, addr: impl Into<SocketAddr>) -> Self {
        self.local_addr = Some(addr.into());
        self
    }

    /// Set capacity of the connection pool for re-useable connection.
    ///
    /// Default to 128
    ///
    /// # Panics:
    /// When pass 0 as pool capacity.
    pub fn set_pool_capacity(mut self, cap: usize) -> Self {
        assert_ne!(cap, 0);
        self.pool_capacity = cap;
        self
    }

    /// Set max http version client would be used.
    ///
    /// Default to Http/3
    pub fn set_max_http_version(mut self, version: Version) -> Self {
        self.max_http_version = version;
        self
    }

    /// Finish the builder and construct [Client] instance.
    pub fn finish(self) -> Client {
        let mut client = {
            #[cfg(feature = "http3")]
            {
                use h3_quinn::quinn::{ClientConfig, Endpoint};

                assert_eq!(
                    self.max_http_version,
                    Version::HTTP_3,
                    "Please disable http3 feature if max_http_version is below HTTP_3"
                );

                #[cfg(not(feature = "dangerous"))]
                let h3_client = {
                    let mut crypto = rustls::ClientConfig::builder().with_safe_defaults();
                    crypto.alpn_protocols = vec![b"h3-29".to_vec()];

                    let config = ClientConfig::new(Arc::new(crypto));

                    let mut endpoint = match self.local_addr {
                        Some(addr) => Endpoint::client(addr).unwrap(),
                        None => Endpoint::client("0.0.0.0:0".parse().unwrap()).unwrap(),
                    };

                    endpoint.set_default_client_config(config);

                    endpoint
                };

                #[cfg(feature = "dangerous")]
                let h3_client = {
                    use std::sync::Arc;

                    use tokio_rustls::rustls;

                    struct SkipServerVerification;

                    impl SkipServerVerification {
                        fn new() -> Arc<Self> {
                            Arc::new(Self)
                        }
                    }
                    impl rustls::client::ServerCertVerifier for SkipServerVerification {
                        fn verify_server_cert(
                            &self,
                            _end_entity: &rustls::Certificate,
                            _intermediates: &[rustls::Certificate],
                            _server_name: &rustls::ServerName,
                            _scts: &mut dyn Iterator<Item = &[u8]>,
                            _ocsp_response: &[u8],
                            _now: std::time::SystemTime,
                        ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
                            Ok(rustls::client::ServerCertVerified::assertion())
                        }
                    }

                    let mut crypto = rustls::ClientConfig::builder()
                        .with_safe_defaults()
                        .with_custom_certificate_verifier(SkipServerVerification::new())
                        .with_no_client_auth();
                    crypto.alpn_protocols = vec![b"h3-29".to_vec()];

                    let config = ClientConfig::new(Arc::new(crypto));

                    let mut endpoint = match self.local_addr {
                        Some(addr) => Endpoint::client(addr).unwrap(),
                        None => Endpoint::client("0.0.0.0:0".parse().unwrap()).unwrap(),
                    };

                    endpoint.set_default_client_config(config);

                    endpoint
                };

                Client {
                    pool: Pool::with_capacity(self.pool_capacity),
                    connector: Connector::default(),
                    resolver: self.resolver,
                    timeout_config: self.timeout_config,
                    local_addr: self.local_addr,
                    date_service: DateTimeService::new(),
                    h3_client,
                }
            }

            #[cfg(not(feature = "http3"))]
            Client {
                pool: Pool::with_capacity(self.pool_capacity),
                connector: Connector::default(),
                resolver: self.resolver,
                timeout_config: self.timeout_config,
                local_addr: self.local_addr,
                date_service: DateTimeService::new(),
            }
        };

        match self.connector_builder {
            TlsConnectorBuilder::Default => {}
            TlsConnectorBuilder::Custom(connector) => client.connector = connector,
            #[cfg(feature = "openssl")]
            TlsConnectorBuilder::Openssl => match self.max_http_version {
                Version::HTTP_09 | Version::HTTP_10 => {
                    unimplemented!("rustls can not be used on HTTP/0.9 nor HTTP/1.0")
                }
                Version::HTTP_11 => {
                    client.connector = Connector::openssl(&[b"http/1.1"]);
                }
                Version::HTTP_2 | Version::HTTP_3 => {
                    client.connector = Connector::openssl(&[b"h2", b"http/1.1"]);
                }
                _ => unreachable!(),
            },
            #[cfg(feature = "rustls")]
            TlsConnectorBuilder::Rustls => match self.max_http_version {
                Version::HTTP_09 | Version::HTTP_10 => {
                    unimplemented!("rustls can not be used on HTTP/0.9 nor HTTP/1.0")
                }
                Version::HTTP_11 => {
                    client.connector = Connector::rustls(&[b"http/1.1"]);
                }
                Version::HTTP_2 | Version::HTTP_3 => {
                    client.connector = Connector::rustls(&[b"h2", b"http/1.1"]);
                }
                _ => unreachable!(),
            },
        };

        client
    }
}

enum TlsConnectorBuilder {
    Default,
    Custom(Connector),
    #[cfg(feature = "openssl")]
    Openssl,
    #[cfg(feature = "rustls")]
    Rustls,
}
