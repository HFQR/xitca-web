use std::{future::Future, marker::PhantomData};

use xitca_io::net;
use xitca_service::{BuildService, BuildServiceExt, EnclosedFactory};

use super::{
    body::RequestBody,
    config::{HttpServiceConfig, DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT},
    error::BuildError,
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
    #[cfg(feature = "http2")]
    pub struct Http2;
}

/// HttpService Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub struct HttpServiceBuilder<
    V,
    St,
    F,
    FA,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    pub(crate) factory: F,
    pub(crate) tls_factory: FA,
    pub(crate) config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    pub(crate) _body: PhantomData<(V, St)>,
}

impl<F>
    HttpServiceBuilder<
        marker::Http,
        net::Stream,
        F,
        tls::NoOpTlsAcceptorService,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    >
{
    /// Construct a new Service Builder with given service factory and default configuration.
    pub const fn new(factory: F) -> Self {
        Self::with_config(factory, HttpServiceConfig::new())
    }

    /// Construct a new Service Builder with given service factory and configuration
    pub const fn with_config<const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>(
        factory: F,
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    ) -> HttpServiceBuilder<
        marker::Http,
        net::Stream,
        F,
        tls::NoOpTlsAcceptorService,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            factory,
            tls_factory: tls::NoOpTlsAcceptorService,
            config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http1")]
    /// Construct a new Http/1 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h1::RequestBody>` as Request type.
    /// This is a request type specific for Http/1 request body.
    pub fn h1(
        factory: F,
    ) -> HttpServiceBuilder<
        marker::Http1,
        net::TcpStream,
        F,
        tls::NoOpTlsAcceptorService,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            factory,
            tls_factory: tls::NoOpTlsAcceptorService,
            config: HttpServiceConfig::default(),
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http2")]
    /// Construct a new Http/2 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h2::RequestBody>` as Request type.
    /// This is a request type specific for Http/2 request body.
    pub fn h2(
        factory: F,
    ) -> HttpServiceBuilder<
        marker::Http2,
        net::TcpStream,
        F,
        tls::NoOpTlsAcceptorService,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            factory,
            tls_factory: tls::NoOpTlsAcceptorService,
            config: HttpServiceConfig::default(),
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http3")]
    /// Construct a new Http/3 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h3::RequestBody>` as Request type.
    /// This is a request type specific for Http/3 request body.
    pub fn h3(factory: F) -> super::h3::H3ServiceBuilder<F> {
        super::h3::H3ServiceBuilder::new(factory)
    }
}

impl<V, St, F, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<V, St, F, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    pub fn config<const HEADER_LIMIT_2: usize, const READ_BUF_LIMIT_2: usize, const WRITE_BUF_LIMIT_2: usize>(
        self,
        config: HttpServiceConfig<HEADER_LIMIT_2, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT_2>,
    ) -> HttpServiceBuilder<V, St, F, FA, HEADER_LIMIT_2, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT_2> {
        HttpServiceBuilder {
            factory: self.factory,
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
    ) -> HttpServiceBuilder<V, St, F, TlsF, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        HttpServiceBuilder {
            factory: self.factory,
            tls_factory,
            config: self.config,
            _body: PhantomData,
        }
    }

    /// Finish builder with default logger.
    ///
    /// Would consume input.
    pub fn with_logger<Arg>(self) -> EnclosedFactory<Self, Logger>
    where
        Self: BuildService<Arg>,
    {
        self.enclosed(Logger::new())
    }

    #[cfg(feature = "openssl")]
    pub fn openssl(
        self,
        acceptor: tls::openssl::TlsAcceptor,
    ) -> HttpServiceBuilder<V, St, F, tls::openssl::TlsAcceptorService, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::openssl::TlsAcceptorService::new(acceptor))
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(
        self,
        config: tls::rustls::RustlsConfig,
    ) -> HttpServiceBuilder<V, St, F, tls::rustls::TlsAcceptorService, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::rustls::TlsAcceptorService::new(config))
    }

    #[cfg(feature = "native-tls")]
    pub fn native_tls(
        self,
        acceptor: tls::native_tls::TlsAcceptor,
    ) -> HttpServiceBuilder<V, St, F, tls::native_tls::TlsAcceptorService, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::native_tls::TlsAcceptorService::new(acceptor))
    }
}

impl<F, Arg, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> BuildService<Arg>
    for HttpServiceBuilder<marker::Http, net::Stream, F, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: BuildService<Arg>,
    FA: BuildService,
{
    type Service =
        HttpService<net::Stream, F::Service, RequestBody, FA::Service, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = BuildError<FA::Error, F::Error>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.factory.build(arg);
        let tls_acceptor = self.tls_factory.build(());
        let config = self.config;

        async move {
            let tls_acceptor = tls_acceptor.await.map_err(BuildError::First)?;
            let service = service.await.map_err(BuildError::Second)?;
            Ok(HttpService::new(config, service, tls_acceptor))
        }
    }
}
