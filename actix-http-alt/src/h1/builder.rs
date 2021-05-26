use std::{future::Future, marker::PhantomData};

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
use super::service::H1Service;
use tokio::net::TcpStream;

/// Http/1 Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub struct H1ServiceBuilder<F, AF> {
    factory: F,
    tls_factory: AF,
}

impl<F, B, E> H1ServiceBuilder<F, tls::NoOpTlsAcceptorFactory>
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
            tls_factory: tls::NoOpTlsAcceptorFactory,
        }
    }
}

impl<F, B, E, AF, TlsSt> H1ServiceBuilder<F, AF>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>>,
    F::Service: 'static,

    AF: ServiceFactory<TcpStream, Response = TlsSt>,
    AF::Service: 'static,
    HttpServiceError: From<AF::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    TlsSt: AsyncRead + AsyncWrite + Unpin,
{
    #[cfg(feature = "openssl")]
    pub fn openssl(self, acceptor: tls::openssl::TlsAcceptor) -> H1ServiceBuilder<F, tls::openssl::TlsAcceptorService> {
        H1ServiceBuilder {
            factory: self.factory,
            tls_factory: tls::openssl::TlsAcceptorService::new(acceptor),
        }
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(
        self,
        config: std::sync::Arc<tls::rustls::ServerConfig>,
    ) -> H1ServiceBuilder<F, tls::rustls::TlsAcceptorService> {
        H1ServiceBuilder {
            factory: self.factory,
            tls_factory: tls::rustls::TlsAcceptorService::new(config),
        }
    }
}

impl<F, B, E, AF, TlsSt> ServiceFactory<TcpStream> for H1ServiceBuilder<F, AF>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>>,
    F::Service: 'static,

    F::Error: ResponseError<F::Response>,

    F::InitError: From<AF::InitError>,

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
    type Service = H1Service<F::Service, (), ()>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let tls_acceptor = self.tls_factory.new_service(());
        async {
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;

            Ok(H1Service::new(service, (), ()))
        }
    }
}
