use core::{net::SocketAddr, time::Duration};

use xitca_http::http::version::Version;

use crate::{
    client::Client,
    connect::Connect,
    date::DateTimeService,
    error::Error,
    middleware, pool,
    resolver::{ResolverService, base_resolver},
    response::Response,
    service::{HttpService, Service, ServiceRequest, async_fn::AsyncFn, http::base_service},
    timeout::TimeoutConfig,
    tls::{
        TlsStream,
        connector::{self, Connector},
    },
};

/// Builder type for [Client]. Offer configurations before a client instance is created.
pub struct ClientBuilder {
    connector: Connector,
    resolver: ResolverService,
    pool_capacity: usize,
    keep_alive_idle: Duration,
    keep_alive_born: Duration,
    timeout_config: TimeoutConfig,
    local_addr: Option<SocketAddr>,
    max_http_version: Version,
    service: HttpService,
    #[cfg(feature = "dangerous")]
    allow_invalid_certificate: bool,
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
            pool_capacity: 2,
            keep_alive_idle: Duration::from_secs(60),
            keep_alive_born: Duration::from_secs(3600),
            timeout_config: TimeoutConfig::new(),
            local_addr: None,
            max_http_version: max_http_version(),
            service: base_service(),
            #[cfg(feature = "dangerous")]
            allow_invalid_certificate: false,
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
    ///     type Response = Response;
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
        S: for<'r, 'c> Service<ServiceRequest<'r, 'c>, Response = Response, Error = Error> + Send + Sync + 'static,
    {
        self.service = Box::new(func(self.service));
        self
    }

    /// add a middleware function to client builder.
    ///
    /// func is an async closure receiving the next middleware in the chain and the incoming request.
    ///
    /// # Examples
    /// ```rust
    /// use xitca_client::{ClientBuilder, Service, ServiceRequest,error::Error};
    /// use xitca_http::http::HeaderValue;
    ///
    /// // start a new client builder and apply our middleware to it:
    /// let builder = ClientBuilder::new()
    ///     // use an async closure to receive HttpService and construct my middleware type.
    ///     .middleware_fn(async |mut req, http_service| {
    ///         req.req.headers_mut().insert("x-my-header", HeaderValue::from_static("my-value"));
    ///         http_service.call(req).await
    ///     });
    /// ```
    pub fn middleware_fn<F>(mut self, func: F) -> Self
    where
        // bound to std::ops::AsyncFn allows us avoid explicit annotating closure argument types
        F: core::ops::AsyncFn(ServiceRequest<'_, '_>, &HttpService) -> Result<Response, Error>,
        // bound to homebrew AsyncFn allows us express `Send` bound to the returned future type
        F: for<'s, 'r, 'c> AsyncFn<(ServiceRequest<'r, 'c>, &'s HttpService), Output = Result<Response, Error>>
            + Send
            + Sync
            + 'static,
        for<'s, 'r, 'c> <F as AsyncFn<(ServiceRequest<'r, 'c>, &'s HttpService)>>::Future: Send,
    {
        self.service = Box::new(middleware::AsyncFn::new(self.service, func));
        self
    }

    #[cfg(feature = "openssl")]
    /// enable openssl as tls connector.
    pub fn openssl(mut self) -> Self {
        self.connector = connector::openssl::connect(
            self.alpn_from_version(),
            #[cfg(feature = "dangerous")]
            self.allow_invalid_certificate,
        );
        self
    }

    #[cfg(any(feature = "rustls", feature = "rustls-ring-crypto"))]
    /// enable rustls as tls connector.
    pub fn rustls(mut self) -> Self {
        self.connector = connector::rustls::connect(
            self.alpn_from_version(),
            #[cfg(feature = "dangerous")]
            self.allow_invalid_certificate,
        );
        self
    }

    #[cfg(any(feature = "openssl", feature = "rustls", feature = "rustls-ring-crypto"))]
    const fn alpn_from_version(&self) -> &[&[u8]] {
        match self.max_http_version {
            Version::HTTP_09 | Version::HTTP_10 => panic!("tls can not be enabled on HTTP/0.9 nor HTTP/1.0"),
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
    /// use xitca_client::{error::Error, http::Version, ClientBuilder, TlsStream, Service};
    ///
    /// // custom connector type
    /// struct MyConnector;
    ///     
    /// // impl trait for connector.
    /// impl<'n> Service<(&'n str, TlsStream)> for MyConnector {
    ///     // expected output types.
    ///     type Response = (TlsStream, Version);
    ///     // potential error type.
    ///     type Error = Error;
    ///
    ///     // name is string representation of server name.
    ///     // box io is type erased generic io type. it can be TcpStream, UnixStream, etc.
    ///     async fn call(&self, (name, io): (&'n str, TlsStream)) -> Result<Self::Response, Self::Error> {
    ///         // tls handshake logic
    ///         // after tls connected must return another type erase io type and according http version of this connection.
    ///         Ok((io, Version::HTTP_11))
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
        T: for<'n> Service<(&'n str, TlsStream), Response = (TlsStream, Version), Error = Error>
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
    /// # Note
    /// capacity is for concurrent opening sockets PER remote Domain.
    /// capacity only applies to http/1 protocol.
    /// http/2 always open one socket per remote domain.
    /// http/3 always open one socket for all remote domains.
    ///
    /// Default to 2
    ///
    /// # Panics
    /// When pass 0 as pool capacity.
    pub fn set_pool_capacity(mut self, cap: usize) -> Self {
        assert_ne!(cap, 0);
        self.pool_capacity = cap;
        self
    }

    /// Set duration of keep alive idle connection.
    ///
    /// This duration force a connection to be closed if it's idle for this long.
    ///
    /// Default to 10 minutes.
    ///
    pub fn set_keep_alive_idle(mut self, duration: Duration) -> Self {
        self.keep_alive_idle = duration;
        self
    }

    /// Set duration of keep alive born connection.
    ///
    /// This duration force a connection to be closed if it has been created for this long.
    ///
    /// Default to 1 hour.
    ///
    pub fn set_keep_alive_born(mut self, duration: Duration) -> Self {
        self.keep_alive_born = duration;
        self
    }

    /// Set max http version client would be used.
    ///
    /// Default to the max version of http feature enabled within Cargo.toml
    ///
    /// # Panics
    /// - panic when given http version is not compat with enabled crate features.
    /// ```no_run
    /// // import xitca-client in Cargo.toml with default features where only http1 is enabled.
    /// // #[dependencies]
    /// // xitca-client = { version = "*" }
    ///
    /// fn config(mut builder: xitca_client::ClientBuilder) {
    /// // trying to set max http version beyond http1 would cause panic.
    ///     builder
    ///         .set_max_http_version(xitca_client::http::Version::HTTP_2)
    ///         .set_max_http_version(xitca_client::http::Version::HTTP_3);
    /// }
    ///
    /// // add additive http features and the panic would be gone.
    /// // #[dependencies]
    /// // xitca-client = { version = "*", features = ["http2", "http3"] }
    /// ```
    pub fn set_max_http_version(mut self, version: Version) -> Self {
        version_check(version);
        self.max_http_version = version;
        self
    }

    #[cfg(feature = "dangerous")]
    /// Allow invalid certificate for tls connection.
    pub fn allow_invalid_certificate(mut self) -> Self {
        self.allow_invalid_certificate = true;
        self
    }

    /// Finish the builder and construct [Client] instance.
    pub fn finish(self) -> Client {
        #[cfg(feature = "http3")]
        let h3_client = {
            use std::sync::Arc;

            use h3_quinn::quinn::Endpoint;
            use webpki_roots::TLS_SERVER_ROOTS;
            use xitca_tls::rustls::{ClientConfig, RootCertStore};

            let mut root_store = RootCertStore::empty();

            root_store.extend(TLS_SERVER_ROOTS.iter().cloned());

            let mut cfg = ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();

            cfg.alpn_protocols = vec![b"h3".to_vec(), b"h32-29".to_vec()];

            #[cfg(feature = "dangerous")]
            {
                if self.allow_invalid_certificate {
                    cfg.dangerous()
                        .set_certificate_verifier(crate::tls::dangerous::rustls::SkipServerVerification::new());
                }
            }

            let mut endpoint = match self.local_addr {
                Some(addr) => Endpoint::client(addr).unwrap(),
                None => Endpoint::client("0.0.0.0:0".parse().unwrap()).unwrap(),
            };

            let cfg = quinn::crypto::rustls::QuicClientConfig::try_from(cfg).unwrap();
            endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(cfg)));

            endpoint
        };

        Client {
            exclusive_pool: pool::exclusive::Pool::new(self.pool_capacity, self.keep_alive_idle, self.keep_alive_born),
            shared_pool: pool::shared::Pool::with_capacity(self.pool_capacity),
            connector: self.connector,
            resolver: self.resolver,
            timeout_config: self.timeout_config,
            max_http_version: self.max_http_version,
            local_addr: self.local_addr,
            date_service: DateTimeService::new(),
            service: self.service,
            #[cfg(feature = "http3")]
            h3_client,
        }
    }
}

pub(crate) fn version_check(version: Version) {
    match (max_http_version(), version) {
        (Version::HTTP_3, _) => {}
        (Version::HTTP_2, Version::HTTP_2 | Version::HTTP_11 | Version::HTTP_10 | Version::HTTP_09) => {}
        (Version::HTTP_11, Version::HTTP_11 | Version::HTTP_10 | Version::HTTP_09) => {}
        (Version::HTTP_10, Version::HTTP_10 | Version::HTTP_09) => {}
        (Version::HTTP_09, Version::HTTP_09) => {}
        _ => panic!("http version: {version:?} is not compat with crate feature setup."),
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
