use std::{fmt, future::Future, marker::PhantomData};

use bytes::Bytes;
use futures_core::Stream;
use http::{Request, Response};
use xitca_server::net::Stream as ServerStream;
use xitca_service::ServiceFactory;

use super::body::{RequestBody, ResponseBody};
use super::config::{HttpServiceConfig, DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT};
use super::error::{BodyError, HttpServiceError};
use super::expect::ExpectHandler;
use super::service::HttpService;
use super::tls::{self, TlsStream};
use super::upgrade::UpgradeHandler;

/// HttpService Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub struct HttpServiceBuilder<
    F,
    ReqB,
    FE,
    FU,
    FA,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    pub(crate) factory: F,
    pub(crate) expect: FE,
    pub(crate) upgrade: Option<FU>,
    pub(crate) tls_factory: FA,
    pub(crate) config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    pub(crate) _body: PhantomData<ReqB>,
}

impl<F>
    HttpServiceBuilder<
        F,
        RequestBody,
        ExpectHandler<F>,
        UpgradeHandler,
        tls::TlsAcceptorService,
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
        F,
        RequestBody,
        ExpectHandler<F>,
        UpgradeHandler,
        tls::TlsAcceptorService,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            factory,
            expect: ExpectHandler::new(),
            upgrade: None,
            tls_factory: tls::TlsAcceptorService::new(),
            config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http1")]
    /// Construct a new Http/1 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h1::RequestBody>` as Request type.
    /// This is a request type specific for Http/1 request body.
    pub fn h1<ResB, E>(
        factory: F,
    ) -> HttpServiceBuilder<
        F,
        super::h1::RequestBody,
        ExpectHandler<F>,
        UpgradeHandler,
        tls::NoOpTlsAcceptorService,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    >
    where
        F: ServiceFactory<Request<super::h1::RequestBody>, Response = Response<ResponseBody<ResB>>, Config = ()>,
        F::Service: 'static,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        HttpServiceBuilder {
            factory,
            expect: ExpectHandler::new(),
            upgrade: None,
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
    pub fn h2<ResB, E>(
        factory: F,
    ) -> HttpServiceBuilder<
        F,
        super::h2::RequestBody,
        (),
        (),
        tls::NoOpTlsAcceptorService,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    >
    where
        F: ServiceFactory<Request<super::h2::RequestBody>, Response = Response<ResponseBody<ResB>>, Config = ()>,
        F::Service: 'static,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        HttpServiceBuilder {
            factory,
            expect: (),
            upgrade: None,
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
    pub fn h3<ResB, E>(factory: F) -> super::h3::H3ServiceBuilder<F>
    where
        F: ServiceFactory<Request<super::h3::RequestBody>, Response = Response<ResponseBody<ResB>>, Config = ()>,
        F::Service: 'static,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        super::h3::H3ServiceBuilder::new(factory)
    }
}

impl<F, ReqB, FE, FU, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<F, ReqB, FE, FU, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    pub fn config<const HEADER_LIMIT_2: usize, const READ_BUF_LIMIT_2: usize, const WRITE_BUF_LIMIT_2: usize>(
        self,
        config: HttpServiceConfig<HEADER_LIMIT_2, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT_2>,
    ) -> HttpServiceBuilder<F, ReqB, FE, FU, FA, HEADER_LIMIT_2, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT_2> {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade: self.upgrade,
            tls_factory: self.tls_factory,
            config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http1")]
    pub fn expect<FE2, ResB>(
        self,
        expect: FE2,
    ) -> HttpServiceBuilder<F, ReqB, FE2, FU, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    where
        FE2: ServiceFactory<Request<ReqB>, Response = Request<ResB>>,
        FE2::Service: 'static,
    {
        HttpServiceBuilder {
            factory: self.factory,
            expect,
            upgrade: self.upgrade,
            tls_factory: self.tls_factory,
            config: self.config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http1")]
    pub fn upgrade<FU2, ResB>(
        self,
        upgrade: FU2,
    ) -> HttpServiceBuilder<F, ReqB, FE, FU2, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    where
        FU2: ServiceFactory<Request<ReqB>, Response = ()>,
        FU2::Service: 'static,
    {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade: Some(upgrade),
            tls_factory: self.tls_factory,
            config: self.config,
            _body: PhantomData,
        }
    }
}

impl<F, FE, FU, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<F, RequestBody, FE, FU, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    #[cfg(feature = "openssl")]
    pub fn openssl(
        self,
        acceptor: tls::openssl::TlsAcceptor,
    ) -> HttpServiceBuilder<
        F,
        RequestBody,
        FE,
        FU,
        tls::TlsAcceptorService,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade: self.upgrade,
            tls_factory: tls::TlsAcceptorService::OpenSsl(tls::openssl::TlsAcceptorService::new(acceptor)),
            config: self.config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(
        self,
        config: tls::rustls::RustlsConfig,
    ) -> HttpServiceBuilder<
        F,
        RequestBody,
        FE,
        FU,
        tls::TlsAcceptorService,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade: self.upgrade,
            tls_factory: tls::TlsAcceptorService::Rustls(tls::rustls::TlsAcceptorService::new(config)),
            config: self.config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "native-tls")]
    pub fn native_tls(
        self,
        acceptor: tls::native_tls::TlsAcceptor,
    ) -> HttpServiceBuilder<
        F,
        RequestBody,
        FE,
        FU,
        tls::TlsAcceptorService,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade: self.upgrade,
            tls_factory: tls::TlsAcceptorService::NativeTls(tls::native_tls::TlsAcceptorService::new(acceptor)),
            config: self.config,
            _body: PhantomData,
        }
    }
}

impl<F, ResB, E, FE, FU, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    ServiceFactory<ServerStream>
    for HttpServiceBuilder<F, RequestBody, FE, FU, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<ResB>>>,
    F::Service: 'static,
    F::Error: fmt::Debug,
    F::InitError: From<FE::InitError> + From<FU::InitError> + From<FA::InitError>,

    // TODO: use a meaningful config.
    FE: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>, Config = ()>,
    FE::Service: 'static,

    // TODO: use a meaningful config.
    FU: ServiceFactory<Request<RequestBody>, Response = (), Config = ()>,
    FU::Service: 'static,

    FA: ServiceFactory<ServerStream, Response = TlsStream, Config = ()>,
    FA::Service: 'static,

    HttpServiceError<F::Error>: From<FU::Error> + From<FA::Error>,
    F::Error: From<FE::Error>,

    ResB: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    type Response = ();
    type Error = HttpServiceError<F::Error>;
    type Config = F::Config;
    type Service = HttpService<
        F::Service,
        RequestBody,
        FE::Service,
        FU::Service,
        FA::Service,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    >;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let expect = self.expect.new_service(());
        let upgrade = self.upgrade.as_ref().map(|upgrade| upgrade.new_service(()));
        let service = self.factory.new_service(cfg);
        let tls_acceptor = self.tls_factory.new_service(());
        let config = self.config;

        async move {
            let expect = expect.await?;
            let upgrade = match upgrade {
                Some(upgrade) => Some(upgrade.await?),
                None => None,
            };
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;

            Ok(HttpService::new(config, service, expect, upgrade, tls_acceptor))
        }
    }
}
