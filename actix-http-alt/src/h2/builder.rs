use std::{future::Future, marker::PhantomData};

use actix_service_alt::ServiceFactory;
use bytes::Bytes;
use futures_core::Stream;
use http::{Request, Response};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::body::ResponseBody;
use crate::builder::HttpServiceBuilder;
use crate::error::{BodyError, HttpServiceError};
use crate::response::ResponseError;

use super::body::RequestBody;
use super::service::H2Service;

/// Http/1 Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub type H2ServiceBuilder<F, FE, FU, FA> = HttpServiceBuilder<F, RequestBody, FE, FU, FA>;

impl<F, FE, FU, FA> HttpServiceBuilder<F, RequestBody, FE, FU, FA> {
    #[cfg(feature = "openssl")]
    pub fn openssl(
        self,
        acceptor: actix_tls_alt::accept::openssl::TlsAcceptor,
    ) -> H2ServiceBuilder<F, FE, FU, actix_tls_alt::accept::openssl::TlsAcceptorService> {
        H2ServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade: self.upgrade,
            tls_factory: actix_tls_alt::accept::openssl::TlsAcceptorService::new(acceptor),
            config: self.config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(
        self,
        config: actix_tls_alt::accept::rustls::RustlsConfig,
    ) -> H2ServiceBuilder<F, FE, FU, actix_tls_alt::accept::rustls::TlsAcceptorService> {
        H2ServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            upgrade: self.upgrade,
            tls_factory: actix_tls_alt::accept::rustls::TlsAcceptorService::new(config),
            config: self.config,
            _body: PhantomData,
        }
    }
}

// /// Http/2 Builder type.
// /// Take in generic types of ServiceFactory for http and tls.
// pub struct H2ServiceBuilder<F, AF> {
//     factory: F,
//     tls_factory: AF,
// }
//
// impl<F, B, E> H2ServiceBuilder<F, tls::NoOpTlsAcceptorService>
// where
//     F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>, Config = ()>,
//     F::Service: 'static,
//
//     B: Stream<Item = Result<Bytes, E>> + 'static,
//     E: 'static,
//     BodyError: From<E>,
// {
//     /// Construct a new Service Builder with given service factory.
//     pub fn new(factory: F) -> Self {
//         Self {
//             factory,
//             tls_factory: tls::NoOpTlsAcceptorService,
//         }
//     }
// }
//
// impl<F, B, E, AF> H2ServiceBuilder<F, AF>
// where
//     F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>>,
//     F::Service: 'static,
//
//     B: Stream<Item = Result<Bytes, E>> + 'static,
//     E: 'static,
//     BodyError: From<E>,
// {
//     #[cfg(feature = "openssl")]
//     pub fn openssl(
//         self,
//         acceptor: actix_tls_alt::accept::openssl::TlsAcceptor,
//     ) -> H2ServiceBuilder<F, actix_tls_alt::accept::openssl::TlsAcceptorService> {
//         H2ServiceBuilder {
//             factory: self.factory,
//             tls_factory: actix_tls_alt::accept::openssl::TlsAcceptorService::new(acceptor),
//         }
//     }
//
//     #[cfg(feature = "rustls")]
//     pub fn rustls(
//         self,
//         config: actix_tls_alt::accept::rustls::RustlsConfig,
//     ) -> H2ServiceBuilder<F, actix_tls_alt::accept::rustls::TlsAcceptorService> {
//         H2ServiceBuilder {
//             factory: self.factory,
//             tls_factory: actix_tls_alt::accept::rustls::TlsAcceptorService::new(config),
//         }
//     }
// }

impl<St, F, B, E, FE, FU, FA, TlsSt> ServiceFactory<St> for H2ServiceBuilder<F, FE, FU, FA>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>>,
    F::Service: 'static,

    F::Error: ResponseError<F::Response>,

    F::InitError: From<FA::InitError>,

    FA: ServiceFactory<St, Response = TlsSt, Config = ()>,
    FA::Service: 'static,
    HttpServiceError: From<FA::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin,
    TlsSt: AsyncRead + AsyncWrite + Unpin,
{
    type Response = ();
    type Error = HttpServiceError;
    type Config = F::Config;
    type Service = H2Service<F::Service, FA::Service>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let tls_acceptor = self.tls_factory.new_service(());
        let config = self.config;

        async move {
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;

            Ok(H2Service::new(config, service, (), (), tls_acceptor))
        }
    }
}
