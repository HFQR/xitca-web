use std::{future::Future, marker::PhantomData};

use actix_server_alt::net::{AsyncReadWrite, Protocol};
use actix_service_alt::ServiceFactory;
use bytes::Bytes;
use futures_core::Stream;
use http::{Request, Response};

use super::body::{RequestBody, ResponseBody};
use super::config::HttpServiceConfig;
use super::error::{BodyError, HttpServiceError};
use super::h1::ExpectHandler;
use super::response::ResponseError;
use super::service::HttpService;
use super::tls;

/// HttpService Builder type.
/// Take in generic types of ServiceFactory for http and tls.
//TODO: use real upgrade service.
pub struct HttpServiceBuilder<F, ReqB, FE = ExpectHandler<F>, FU = ExpectHandler<F>, FA = tls::NoOpTlsAcceptorFactory> {
    pub(crate) factory: F,
    pub(crate) expect: FE,
    pub(crate) upgrade: FU,
    pub(crate) tls_factory: FA,
    pub(crate) config: HttpServiceConfig,
    _body: PhantomData<ReqB>,
}

impl<F> HttpServiceBuilder<F, RequestBody> {
    #[cfg(feature = "http1")]
    /// Construct a new Http/1 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h1::RequestBody>` as Request type.
    /// This is a request type specific for Http/1 request body.
    pub fn h1<ResB, E>(factory: F) -> HttpServiceBuilder<F, super::h1::RequestBody>
    where
        F: ServiceFactory<Request<super::h1::RequestBody>, Response = Response<ResponseBody<ResB>>, Config = ()>,
        F::Service: 'static,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        HttpServiceBuilder::new(factory)
    }

    #[cfg(feature = "http2")]
    /// Construct a new Http/2 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h2::RequestBody>` as Request type.
    /// This is a request type specific for Http/2 request body.
    pub fn h2<ResB, E>(factory: F) -> super::h2::H2ServiceBuilder<F, tls::NoOpTlsAcceptorFactory>
    where
        F: ServiceFactory<Request<super::h2::RequestBody>, Response = Response<ResponseBody<ResB>>, Config = ()>,
        F::Service: 'static,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        super::h2::H2ServiceBuilder::new(factory)
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

impl<F, ReqB> HttpServiceBuilder<F, ReqB> {
    /// Construct a new Service Builder with given service factory and default configuration.
    pub fn new(factory: F) -> Self {
        Self::with_config(factory, HttpServiceConfig::default())
    }

    /// Construct a new Service Builder with given service factory and configuration
    pub fn with_config(factory: F, config: HttpServiceConfig) -> Self {
        Self {
            factory,
            expect: ExpectHandler::new(),
            upgrade: ExpectHandler::new(),
            tls_factory: tls::NoOpTlsAcceptorFactory,
            config,
            _body: PhantomData,
        }
    }
}

impl<F, ReqB, FE, FU, FA> HttpServiceBuilder<F, ReqB, FE, FU, FA> {
    pub fn config(mut self, config: HttpServiceConfig) -> Self {
        self.config = config;
        self
    }

    pub fn expect<FE2, ResB>(self, expect: FE2) -> HttpServiceBuilder<F, ReqB, FE2, FU, FA>
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

    pub fn upgrade<FU2, ResB>(self, upgrade: FU2) -> HttpServiceBuilder<F, ReqB, FE, FU2, FA>
    where
        FU2: ServiceFactory<Request<ReqB>, Response = Request<ResB>>,
        FU2::Service: 'static,
    {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade,
            tls_factory: self.tls_factory,
            config: self.config,
            _body: PhantomData,
        }
    }
}

impl<F, ReqB, FE, FU, FA> HttpServiceBuilder<F, ReqB, FE, FU, FA> {
    #[cfg(feature = "openssl")]
    pub fn openssl(
        self,
        acceptor: actix_tls_alt::accept::openssl::TlsAcceptor,
    ) -> HttpServiceBuilder<F, ReqB, FE, FU, actix_tls_alt::accept::openssl::TlsAcceptorService> {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade: self.upgrade,
            tls_factory: actix_tls_alt::accept::openssl::TlsAcceptorService::new(acceptor),
            config: self.config,
            _body: PhantomData,
        }
    }
}

// impl<F, B, E, EF, AF, TlsSt> HttpServiceBuilder<F, EF, AF>
// // where
// //     F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>>,
// //     F::Service: 'static,
// //
// //     EF: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>>,
// //     EF::Service: 'static,
// //
// //     AF: ServiceFactory<TcpStream, Response = TlsSt>,
// //     AF::Service: 'static,
// //     HttpServiceError: From<AF::Error>,
// //
// //     B: Stream<Item = Result<Bytes, E>> + 'static,
// //     E: 'static,
// //     BodyError: From<E>,
// //
// //     TlsSt: AsyncRead + AsyncWrite + Unpin,
//
//     #[cfg(feature = "rustls")]
//     pub fn rustls(
//         self,
//         config: std::sync::Arc<tls::rustls::ServerConfig>,
//     ) -> H1ServiceBuilder<F, EF, tls::rustls::TlsAcceptorService> {
//         H1ServiceBuilder {
//             factory: self.factory,
//             expect: self.expect,
//             tls_factory: tls::rustls::TlsAcceptorService::new(config),
//             config: self.config,
//         }
//     }
// }
//

impl<St, F, ResB, E, FE, FU, FA, TlsSt> ServiceFactory<St> for HttpServiceBuilder<F, RequestBody, FE, FU, FA>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<ResB>>>,
    F::Service: 'static,
    F::Error: ResponseError<F::Response>,
    F::InitError: From<FE::InitError> + From<FU::InitError> + From<FA::InitError>,

    // TODO: use a meaningful config.
    FE: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>, Config = ()>,
    FE::Service: 'static,
    FE::Error: ResponseError<F::Response>,

    // TODO: use a meaningful config and a real upgrade service.
    FU: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>, Config = ()>,
    FU::Service: 'static,
    FU::Error: ResponseError<F::Response>,

    FA: ServiceFactory<St, Response = (TlsSt, Protocol), Config = ()>,
    FA::Service: 'static,
    HttpServiceError: From<FA::Error>,

    ResB: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncReadWrite,
    TlsSt: AsyncReadWrite,
{
    type Response = ();
    type Error = HttpServiceError;
    type Config = F::Config;
    type Service = HttpService<F::Service, RequestBody, FE::Service, FU::Service, FA::Service>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let expect = self.expect.new_service(());
        let upgrade = self.upgrade.new_service(());
        let service = self.factory.new_service(cfg);
        let tls_acceptor = self.tls_factory.new_service(());
        let config = self.config;

        async move {
            let expect = expect.await?;
            let upgrade = upgrade.await?;
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;

            Ok(HttpService::new(config, service, expect, upgrade, tls_acceptor))
        }
    }
}
