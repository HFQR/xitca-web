use std::{future::Future, marker::PhantomData};

use bytes::Bytes;
use http::Response;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::service::ServiceFactory;

use super::error::HttpServiceError;
use super::h2::H2Service;
use super::request::HttpRequest;
use super::tls::NoOpTlsAcceptorFactory;

/// HttpService Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub struct HttpServiceBuilder<St, F, AF, TlsSt> {
    factory: F,
    tls_factory: AF,
    _phantom: PhantomData<(St, TlsSt)>,
}

impl<St, F> HttpServiceBuilder<St, F, NoOpTlsAcceptorFactory, St>
where
    F: ServiceFactory<HttpRequest<super::h2::RequestBody>, Response = Response<Bytes>, Config = ()>,
    F::Service: 'static,

    St: AsyncRead + AsyncWrite + Unpin + 'static,
{
    /// Construct a new Service Builder with given service factory.
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            tls_factory: NoOpTlsAcceptorFactory,
            _phantom: PhantomData,
        }
    }
}

use super::tls::openssl::{TlsAcceptor, TlsAcceptorService, TlsStream};

impl<St, F, AF, TlsSt> HttpServiceBuilder<St, F, AF, TlsSt>
where
    F: ServiceFactory<HttpRequest<super::h2::RequestBody>, Response = Response<Bytes>, Config = ()>,
    F::Service: 'static,

    AF: ServiceFactory<St, Response = TlsSt, Config = ()>,
    AF::Service: 'static,

    St: AsyncRead + AsyncWrite + Unpin + 'static,
    TlsSt: AsyncRead + AsyncWrite + Unpin + 'static,

    HttpServiceError: From<AF::Error>,
{
    pub fn openssl(
        self,
        acceptor: TlsAcceptor,
    ) -> HttpServiceBuilder<St, F, TlsAcceptorService<St>, TlsStream<St>> {
        HttpServiceBuilder {
            factory: self.factory,
            tls_factory: TlsAcceptorService::new(acceptor),
            _phantom: PhantomData,
        }
    }
}

impl<St, F, AF, TlsSt> ServiceFactory<St> for HttpServiceBuilder<St, F, AF, TlsSt>
where
    F: ServiceFactory<HttpRequest<super::h2::RequestBody>, Response = Response<Bytes>, Config = ()>,
    F::Service: 'static,

    AF: ServiceFactory<St, Response = TlsSt, Config = ()>,
    AF::Service: 'static,

    St: AsyncRead + AsyncWrite + Unpin + 'static,
    TlsSt: AsyncRead + AsyncWrite + Unpin + 'static,

    HttpServiceError: From<AF::Error>,
{
    type Response = ();
    type Error = HttpServiceError;
    type Config = ();
    type Service = H2Service<St, F::Service, AF::Service, TlsSt>;
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
