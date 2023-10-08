use std::{future::Future, marker::PhantomData};

use xitca_io::net;
use xitca_service::{EnclosedFactory, Service, ServiceExt};

use super::{
    body::RequestBody,
    config::{HttpServiceConfig, DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT},
    service::HttpService,
    tls,
    util::middleware::Logger,
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
    #[cfg(feature = "http2")]
    pub struct Http2;
}

/// HttpService Builder type.
/// Take in generic types of ServiceFactory for http and tls.
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
    pub(crate) _body: PhantomData<(V, St)>,
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
    /// Construct a new Service Builder with given service factory and default configuration.
    pub const fn new() -> Self {
        Self::with_config(HttpServiceConfig::new())
    }

    /// Construct a new Service Builder with given service factory and configuration
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
    /// Construct a new Http/1 ServiceBuilder.
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
    /// Construct a new Http/2 ServiceBuilder.
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
    /// Construct a new Http/3 ServiceBuilder.
    pub fn h3() -> super::h3::H3ServiceBuilder {
        super::h3::H3ServiceBuilder::new()
    }
}

impl<V, St, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<V, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
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

    /// Pass a tls service factory to Builder and use it to handle tls handling with
    /// Http/1 and Http/2.
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

    /// Finish builder with default logger.
    ///
    /// Would consume input.
    pub fn with_logger(self) -> EnclosedFactory<Self, Logger>
    where
        Self: Service,
    {
        self.enclosed(Logger::new())
    }

    #[cfg(feature = "openssl")]
    pub fn openssl(
        self,
        acceptor: tls::openssl::TlsAcceptor,
    ) -> HttpServiceBuilder<V, St, tls::openssl::TlsAcceptorBuilder, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::openssl::TlsAcceptorBuilder::new(acceptor))
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(
        self,
        config: tls::rustls::RustlsConfig,
    ) -> HttpServiceBuilder<V, St, tls::rustls::TlsAcceptorBuilder, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        self.with_tls(tls::rustls::TlsAcceptorBuilder::new(config))
    }

    #[cfg(feature = "rustls-uring")]
    pub fn rustls_uring(
        self,
        config: tls::rustls::RustlsConfig,
    ) -> HttpServiceBuilder<V, St, tls::rustls_uring::TlsAcceptorBuilder, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::rustls_uring::TlsAcceptorBuilder::new(config))
    }

    #[cfg(feature = "native-tls")]
    pub fn native_tls(
        self,
        acceptor: tls::native_tls::TlsAcceptor,
    ) -> HttpServiceBuilder<V, St, tls::native_tls::TlsAcceptorBuilder, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::native_tls::TlsAcceptorBuilder::new(acceptor))
    }
}

impl<S, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<S>
    for HttpServiceBuilder<marker::Http, net::Stream, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    FA: Service,
{
    type Response =
        HttpService<net::Stream, S, RequestBody, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = FA::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, S: 'f;

    fn call<'s>(&'s self, service: S) -> Self::Future<'s>
    where
        S: 's,
    {
        async {
            let tls_acceptor = self.tls_factory.call(()).await?;
            Ok(HttpService::new(self.config, service, tls_acceptor))
        }
    }
}
