use std::future::Future;

use actix_server_alt::net::TcpStream;
use actix_service_alt::ServiceFactory;
use bytes::Bytes;
use futures_core::Stream;
use http::{Request, Response};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::response::ResponseError;
use crate::tls;

use super::body::RequestBody;
use super::expect::ExpectHandler;
use super::service::H1Service;

/// Http/1 Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub struct H1ServiceBuilder<F, EF = ExpectHandler<F>, AF = tls::NoOpTlsAcceptorFactory> {
    factory: F,
    expect: EF,
    tls_factory: AF,
}

impl<F, B, E> H1ServiceBuilder<F>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>, Config = ()>,
    F::Service: 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    /// Construct a new Service Builder with given service factory.
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            expect: ExpectHandler::new(),
            tls_factory: tls::NoOpTlsAcceptorFactory,
        }
    }
}

impl<F, B, E, EF, AF, TlsSt> H1ServiceBuilder<F, EF, AF>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>>,
    F::Service: 'static,

    EF: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>>,
    EF::Service: 'static,

    AF: ServiceFactory<TcpStream, Response = TlsSt>,
    AF::Service: 'static,
    HttpServiceError: From<AF::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    TlsSt: AsyncRead + AsyncWrite + Unpin,
{
    pub fn expect<EF2>(self, expect: EF2) -> H1ServiceBuilder<F, EF2, AF>
    where
        EF2: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>>,
        EF2::Service: 'static,
    {
        H1ServiceBuilder {
            factory: self.factory,
            expect,
            tls_factory: self.tls_factory,
        }
    }

    #[cfg(feature = "openssl")]
    pub fn openssl(
        self,
        acceptor: tls::openssl::TlsAcceptor,
    ) -> H1ServiceBuilder<F, EF, tls::openssl::TlsAcceptorService> {
        H1ServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            tls_factory: tls::openssl::TlsAcceptorService::new(acceptor),
        }
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(
        self,
        config: std::sync::Arc<tls::rustls::ServerConfig>,
    ) -> H1ServiceBuilder<F, EF, tls::rustls::TlsAcceptorService> {
        H1ServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            tls_factory: tls::rustls::TlsAcceptorService::new(config),
        }
    }
}

impl<F, B, E, EF, AF, TlsSt> ServiceFactory<TcpStream> for H1ServiceBuilder<F, EF, AF>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>>,
    F::Service: 'static,
    F::Error: ResponseError<F::Response>,
    F::InitError: From<AF::InitError> + From<EF::InitError>,

    // TODO: use a meaningful config.
    EF: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>, Config = ()>,
    EF::Service: 'static,

    AF: ServiceFactory<TcpStream, Response = TlsSt, Config = ()>,
    AF::Service: 'static,
    HttpServiceError: From<AF::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    TlsSt: AsyncRead + AsyncWrite + Unpin,
{
    type Response = ();
    type Error = HttpServiceError;
    type Config = F::Config;
    type Service = H1Service<F::Service, EF::Service, ()>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let expect = self.expect.new_service(());
        let service = self.factory.new_service(cfg);
        let tls_acceptor = self.tls_factory.new_service(());
        async {
            let expect = expect.await?;
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;
            Ok(H1Service::new(service, expect, ()))
        }
    }
}
