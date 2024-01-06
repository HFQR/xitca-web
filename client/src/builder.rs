use std::{net::SocketAddr, time::Duration};

use xitca_http::http::version::Version;

use crate::{
    client::Client,
    connect::Connect,
    date::DateTimeService,
    error::Error,
    pool::Pool,
    resolver::{base_resolver, ResolverService},
    response::Response,
    service::{base_service, HttpService},
    service::{Service, ServiceRequest},
    timeout::TimeoutConfig,
    tls::{
        connector::{self, Connector},
        stream::Io,
    },
};

/// Builder type for [Client]. Offer configurations before a client instance is created.
pub struct ClientBuilder {
    connector: Connector,
    resolver: ResolverService,
    pool_capacity: usize,
    timeout_config: TimeoutConfig,
    local_addr: Option<SocketAddr>,
    max_http_version: Version,
    service: HttpService,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    pub fn new() -> Self {
        ClientBuilder {
            connector: connector::nop(),
            resolver: base_resolver(),
            pool_capacity: 128,
            timeout_config: TimeoutConfig::new(),
            local_addr: None,
            max_http_version: max_http_version(),
            service: base_service(),
        }
    }

    /// add middleware service to client builder.
    /// middleware is a type impl [Service] trait that take ownership of [HttpService] that
    /// can pre-process [ServiceRequest] and post-process of output of [HttpService] as `Result<Response, Error>`
    ///
    /// # Examples
    /// ```rust
    /// use xitca_client::{
    ///     error::Error,
    ///     ClientBuilder, HttpService, Response, Service, ServiceRequest
    /// };
    ///
    /// // a typed middleware that contains the http service xitca-cleint provides.
    /// struct MyMiddleware {
    ///     // http service contains main logic of making http request and response.
    ///     http_service: HttpService,
    /// }
    ///
    /// // trait implement for the logic of middleware. most of the types are boilerplate
    /// // that can be copy/pasted. the real logic goes into `async fn call`
    /// impl<'r, 'c> Service<ServiceRequest<'r, 'c>> for MyMiddleware {
    ///     type Response = Response<'c>;
    ///     type Error = Error;
    ///
    ///     async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
    ///         // my middleware receive ServiceRequest and can do pre-process before passing it to
    ///         // HttpService. in this case we just print out the HTTP method of request.
    ///         println!("request method is: {}", req.req.method());
    ///
    ///         // after pre-processing of ServiceRequest we pass it to HttpService and execute the
    ///         // processing of request/response logic.
    ///         match self.http_service.call(req).await {
    ///             // after the call method returns we observe the outcome as Result where we can do
    ///             // post-processing.
    ///             Ok(res) => {
    ///                 // Ok branch contains the response we received from server. in this case we just
    ///                 // print out it's status code.
    ///                 println!("response status is: {}", res.status());   
    ///
    ///                 // after post-processing return the response
    ///                 Ok(res)
    ///             }
    ///             Err(e) => {
    ///                 // Err branch contains possible error happens during the HttpService executing.
    ///                 // It can either be caused by local or remote. in this case we just print out the
    ///                 // error.
    ///                 println!("observed error: {}", e);
    ///
    ///                 // after post-processing return the error.
    ///                 Err(e)
    ///             }
    ///         }
    ///     }
    /// }
    ///
    /// // start a new client builder and apply our middleware to it:
    /// let builder = ClientBuilder::new()
    ///     // use a closure to receive HttpService and construct my middleware type.
    ///     .middleware(|http_service| MyMiddleware { http_service });
    /// ```
    pub fn middleware<F, S>(mut self, func: F) -> Self
    where
        F: FnOnce(HttpService) -> S,
        S: for<'r, 'c> Service<ServiceRequest<'r, 'c>, Response = Response<'c>, Error = Error> + Send + Sync + 'static,
    {
        self.service = Box::new(func(self.service));
        self
    }

    #[cfg(feature = "openssl")]
    /// enable openssl as tls connector.
    pub fn openssl(mut self) -> Self {
        self.connector = connector::openssl(self.alpn_from_version());
        self
    }

    #[cfg(feature = "rustls")]
    /// enable rustls as tls connector.
    pub fn rustls(mut self) -> Self {
        self.connector = connector::rustls(self.alpn_from_version());
        self
    }

    #[cfg(any(feature = "openssl", feature = "rustls"))]
    const fn alpn_from_version(&self) -> &[&[u8]] {
        match self.max_http_version {
            Version::HTTP_09 | Version::HTTP_10 => {
                panic!("tls can not be used on HTTP/0.9 nor HTTP/1.0")
            }
            Version::HTTP_11 => &[b"http/1.1"],
            Version::HTTP_2 | Version::HTTP_3 => &[b"h2", b"http/1.1"],
            _ => unreachable!(),
        }
    }

    /// Use custom DNS resolver for domain look up. custom resolver must impl [Service] trait.
    ///
    /// # Example
    /// ```rust
    /// use xitca_client::{error::Error, ClientBuilder, Connect, Service};
    ///
    /// // custom dns resolver type
    /// struct MyResolver;
    ///
    /// // implement trait for dns resolver.
    /// impl<'r, 'c> Service<&'r mut Connect<'c>> for MyResolver {
    ///     type Response = ();
    ///     // possible error type when resolving failed.
    ///     type Error = Error;
    ///
    ///     async fn call(&self, connect: &'r mut Connect<'c>) -> Result<Self::Response, Self::Error> {
    ///         let _host = connect.hostname(); // connect provides host name of domain needs to be resolved.
    ///         let _port = connect.port(); // the same goes for optional port number of domain.
    ///         
    ///         // your dns resolving logic should produce one or multiple std::net::SocketAddr
    ///         let addr = "127.0.0.1".parse().unwrap();
    ///
    ///         // add resolved socket addr(s) to connect which would be used for establishing connections.
    ///         connect.set_addrs([addr]);
    ///
    ///         Ok(())
    ///     }
    /// }
    ///
    /// # fn resolve() {
    /// // apply resolver to client builder.
    /// let client = ClientBuilder::new().resolver(MyResolver).finish();
    /// # }
    /// ```
    pub fn resolver<R>(mut self, resolver: R) -> Self
    where
        R: for<'r, 'c> Service<&'r mut Connect<'c>, Response = (), Error = Error> + Send + Sync + 'static,
    {
        self.resolver = Box::new(resolver);
        self
    }

    /// Use custom tls connector for tls handshaking. custom connector must impl [Service] trait.
    ///
    /// # Examples
    /// ```rust
    /// use xitca_client::{error::Error, http::Version, ClientBuilder, Io, Service};
    ///
    /// // custom connector type
    /// struct MyConnector;
    ///     
    /// // impl trait for connector.
    /// impl<'n> Service<(&'n str, Box<dyn Io>)> for MyConnector {
    ///     // expected output types.
    ///     type Response = (Box<dyn Io>, Version);
    ///     // potential error type.
    ///     type Error = Error;
    ///
    ///     // name is string representation of server name.
    ///     // box io is type erased generic io type. it can be TcpStream, UnixStream, etc.
    ///     async fn call(&self, (name, io): (&'n str, Box<dyn Io>)) -> Result<Self::Response, Self::Error> {
    ///         // tls handshake logic
    ///         // after tls connected must return another type erase io type and according http version of this connection.
    ///         Ok((Box::new(io), Version::HTTP_11))
    ///     }
    /// }
    ///
    /// # fn resolve() {
    /// // apply connector to client builder.
    /// let client = ClientBuilder::new().tls_connector(MyConnector).finish();
    /// # }
    /// ```
    pub fn tls_connector<T>(mut self, connector: T) -> Self
    where
        T: for<'n> Service<(&'n str, Box<dyn Io>), Response = (Box<dyn Io>, Version), Error = Error>
            + Send
            + Sync
            + 'static,
    {
        self.connector = Box::new(connector);
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

    /// Set [TimeoutConfig] for client.
    pub fn set_timeout(mut self, timeout_config: TimeoutConfig) -> Self {
        self.timeout_config = timeout_config;
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
    /// Default to the max version of http feature enabled within Cargo.toml
    ///
    /// # Examples
    /// ```(no_run)
    /// // default max http version would be Version::HTTP_2
    /// xitca-client = { version = "*", features = ["http2"] }
    /// ```
    pub fn set_max_http_version(mut self, version: Version) -> Self {
        self.max_http_version = version;
        self
    }

    /// Finish the builder and construct [Client] instance.
    pub fn finish(self) -> Client {
        #[cfg(feature = "http3")]
        {
            use std::sync::Arc;

            use h3_quinn::quinn::{ClientConfig, Endpoint};
            use rustls_0dot21 as rustls;

            #[cfg(not(feature = "dangerous"))]
            let h3_client = {
                use rustls::{OwnedTrustAnchor, RootCertStore};
                use webpki_roots_0dot25::TLS_SERVER_ROOTS;

                let mut root_certs = RootCertStore::empty();
                for cert in TLS_SERVER_ROOTS {
                    let cert = OwnedTrustAnchor::from_subject_spki_name_constraints(
                        cert.subject,
                        cert.spki,
                        cert.name_constraints,
                    );
                    let certs = vec![cert].into_iter();
                    root_certs.add_trust_anchors(certs);
                }

                let mut crypto = rustls::ClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(root_certs)
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

            #[cfg(feature = "dangerous")]
            let h3_client = {
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
                connector: self.connector,
                resolver: self.resolver,
                timeout_config: self.timeout_config,
                max_http_version: self.max_http_version,
                local_addr: self.local_addr,
                date_service: DateTimeService::new(),
                service: self.service,
                h3_client,
            }
        }

        #[cfg(not(feature = "http3"))]
        Client {
            pool: Pool::with_capacity(self.pool_capacity),
            connector: self.connector,
            resolver: self.resolver,
            timeout_config: self.timeout_config,
            max_http_version: self.max_http_version,
            local_addr: self.local_addr,
            date_service: DateTimeService::new(),
            service: self.service,
        }
    }
}

#[allow(unreachable_code)]
fn max_http_version() -> Version {
    #[cfg(feature = "http3")]
    return Version::HTTP_3;

    #[cfg(feature = "http2")]
    return Version::HTTP_2;

    Version::HTTP_11
}
