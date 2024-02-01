use core::{fmt, marker::PhantomData};

use xitca_io::net;
use xitca_service::Service;

use super::{
    body::RequestBody,
    config::{HttpServiceConfig, DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT},
    service::HttpService,
    tls,
};

// marker type for separate HttpServerBuilders' ServiceFactory implement with specialized trait
// method.
#[doc(hidden)]
pub(crate) mod marker {
    pub struct Http;
    #[cfg(feature = "http1")]
    pub struct Http1;
    #[cfg(all(feature = "io-uring", feature = "http1"))]
    pub struct Http1Uring;
    #[cfg(all(feature = "io-uring", feature = "http2"))]
    pub struct Http2Uring;
    #[cfg(feature = "http2")]
    pub struct Http2;
}

/// HttpService middleware.
/// bridge TCP/UDP traffic and HTTP protocols for [Service] type.
pub struct HttpServiceBuilder<
    V,
    St,
    FA,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    pub(crate) tls_factory: FA,
    pub(crate) config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    pub(crate) _body: PhantomData<fn(V, St)>,
}

impl
    HttpServiceBuilder<
        marker::Http,
        net::Stream,
        tls::NoOpTlsAcceptorBuilder,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    >
{
    /// construct a new service middleware.
    pub const fn new() -> Self {
        Self::with_config(HttpServiceConfig::new())
    }

    /// construct a new service middleware with given [HttpServiceConfig].
    pub const fn with_config<const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>(
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    ) -> HttpServiceBuilder<
        marker::Http,
        net::Stream,
        tls::NoOpTlsAcceptorBuilder,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            tls_factory: tls::NoOpTlsAcceptorBuilder,
            config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http1")]
    /// construct a new service middleware for Http/1 protocol only.
    pub fn h1() -> HttpServiceBuilder<
        marker::Http1,
        net::TcpStream,
        tls::NoOpTlsAcceptorBuilder,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            tls_factory: tls::NoOpTlsAcceptorBuilder,
            config: HttpServiceConfig::default(),
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http2")]
    /// construct a new service middleware for Http/2 protocol only.
    pub fn h2() -> HttpServiceBuilder<
        marker::Http2,
        net::TcpStream,
        tls::NoOpTlsAcceptorBuilder,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            tls_factory: tls::NoOpTlsAcceptorBuilder,
            config: HttpServiceConfig::default(),
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http3")]
    /// construct a new service middleware for Http/3 protocol only.
    pub fn h3() -> super::h3::H3ServiceBuilder {
        super::h3::H3ServiceBuilder::new()
    }
}

impl<V, St, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<V, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// replace service middleware's configuration.
    pub fn config<const HEADER_LIMIT_2: usize, const READ_BUF_LIMIT_2: usize, const WRITE_BUF_LIMIT_2: usize>(
        self,
        config: HttpServiceConfig<HEADER_LIMIT_2, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT_2>,
    ) -> HttpServiceBuilder<V, St, FA, HEADER_LIMIT_2, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT_2> {
        HttpServiceBuilder {
            tls_factory: self.tls_factory,
            config,
            _body: PhantomData,
        }
    }

    /// replace tls service. tls service is used for Http/1 and Http/2 protocols.
    pub fn with_tls<TlsF>(
        self,
        tls_factory: TlsF,
    ) -> HttpServiceBuilder<V, St, TlsF, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        HttpServiceBuilder {
            tls_factory,
            config: self.config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "openssl")]
    /// use openssl as tls service. tls service is used for Http/1 and Http/2 protocols.
    pub fn openssl(
        self,
        acceptor: tls::openssl::TlsAcceptor,
    ) -> HttpServiceBuilder<V, St, tls::openssl::TlsAcceptorBuilder, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::openssl::TlsAcceptorBuilder::new(acceptor))
    }

    #[cfg(feature = "rustls")]
    /// use rustls as tls service. tls service is used for Http/1 and Http/2 protocols.
    pub fn rustls(
        self,
        config: tls::rustls::RustlsConfig,
    ) -> HttpServiceBuilder<V, St, tls::rustls::TlsAcceptorBuilder, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        self.with_tls(tls::rustls::TlsAcceptorBuilder::new(config))
    }

    #[cfg(feature = "rustls-uring")]
    /// use rustls on io-uring as tls service. io-uring (either with or without) is used for Http/1 protocol only.
    pub fn rustls_uring(
        self,
        config: tls::rustls::RustlsConfig,
    ) -> HttpServiceBuilder<V, St, tls::rustls_uring::TlsAcceptorBuilder, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::rustls_uring::TlsAcceptorBuilder::new(config))
    }

    #[cfg(feature = "native-tls")]
    /// use native-tls as tls service. tnative-tlsls service is used for Http/1 protocol only.
    pub fn native_tls(
        self,
        acceptor: tls::native_tls::TlsAcceptor,
    ) -> HttpServiceBuilder<V, St, tls::native_tls::TlsAcceptorBuilder, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::native_tls::TlsAcceptorBuilder::new(acceptor))
    }
}

type Error = Box<dyn fmt::Debug>;

impl<FA, S, E, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<Result<S, E>>
    for HttpServiceBuilder<marker::Http, net::Stream, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    FA: Service,
    FA::Error: fmt::Debug + 'static,
    E: fmt::Debug + 'static,
{
    type Response =
        HttpService<net::Stream, S, RequestBody, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = Error;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        let service = res.map_err(|e| Box::new(e) as Error)?;
        let tls_acceptor = self.tls_factory.call(()).await.map_err(|e| Box::new(e) as Error)?;
        Ok(HttpService::new(self.config, service, tls_acceptor))
    }
}
