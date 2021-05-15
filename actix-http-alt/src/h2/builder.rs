use std::{future::Future, marker::PhantomData};

use actix_service_alt::{Service, ServiceFactory};
use bytes::Bytes;
use futures_core::Stream;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::request::HttpRequest;
use crate::response::{HttpResponse, ResponseError};
use crate::tls;

use super::body::RequestBody;
use super::service::H2Service;

/// Http/2 Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub struct H2ServiceBuilder<St, F, B, AF, TlsSt> {
    factory: F,
    tls_factory: AF,
    _phantom: PhantomData<(St, B, TlsSt)>,
}

impl<St, F, B, E> H2ServiceBuilder<St, F, B, tls::NoOpTlsAcceptorFactory, St>
where
    F: ServiceFactory<
        HttpRequest<RequestBody>,
        Response = HttpResponse<ResponseBody<B>>,
        Config = (),
    >,
    F::Service: 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin + 'static,
{
    /// Construct a new Service Builder with given service factory.
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            tls_factory: tls::NoOpTlsAcceptorFactory,
            _phantom: PhantomData,
        }
    }
}

impl<St, F, B, E, AF, TlsSt> H2ServiceBuilder<St, F, B, AF, TlsSt>
where
    F: ServiceFactory<
        HttpRequest<RequestBody>,
        Response = HttpResponse<ResponseBody<B>>,
        Config = (),
    >,
    F::Service: 'static,

    AF: ServiceFactory<St, Response = TlsSt, Config = ()>,
    AF::Service: 'static,
    HttpServiceError: From<AF::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin + 'static,
    TlsSt: AsyncRead + AsyncWrite + Unpin + 'static,
{
    #[cfg(feature = "openssl")]
    pub fn openssl(
        self,
        acceptor: tls::openssl::TlsAcceptor,
    ) -> H2ServiceBuilder<St, F, B, tls::openssl::TlsAcceptorService<St>, tls::openssl::TlsStream<St>>
    {
        H2ServiceBuilder {
            factory: self.factory,
            tls_factory: tls::openssl::TlsAcceptorService::new(acceptor),
            _phantom: PhantomData,
        }
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(
        self,
        config: std::sync::Arc<tls::rustls::ServerConfig>,
    ) -> H2ServiceBuilder<St, F, B, tls::rustls::TlsAcceptorService<St>, tls::rustls::TlsStream<St>>
    {
        H2ServiceBuilder {
            factory: self.factory,
            tls_factory: tls::rustls::TlsAcceptorService::new(config),
            _phantom: PhantomData,
        }
    }
}

impl<St, F, B, E, AF, TlsSt> ServiceFactory<St> for H2ServiceBuilder<St, F, B, AF, TlsSt>
where
    F: ServiceFactory<
        HttpRequest<RequestBody>,
        Response = HttpResponse<ResponseBody<B>>,
        Config = (),
    >,
    F::Service: 'static,

    <F::Service as Service>::Error: ResponseError<<F::Service as Service>::Response>,

    AF: ServiceFactory<St, Response = TlsSt, Config = ()>,
    AF::Service: 'static,
    HttpServiceError: From<AF::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin + 'static,
    TlsSt: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = ();
    type Error = HttpServiceError;
    type Config = ();
    type Service = H2Service<St, F::Service, B, AF::Service, TlsSt>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let service = self.factory.new_service(());
        let tls_acceptor = self.tls_factory.new_service(());
        async {
            let service = match service.await {
                Ok(service) => service,
                Err(_) => panic!("TODO"),
            };

            let tls_acceptor = match tls_acceptor.await {
                Ok(service) => service,
                Err(_) => panic!("TODO"),
            };

            Ok(H2Service::new(service, tls_acceptor))
        }
    }
}
