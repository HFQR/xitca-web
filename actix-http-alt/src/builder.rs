use std::marker::PhantomData;

use actix_service_alt::ServiceFactory;
use bytes::Bytes;
use futures_core::Stream;
use http::{Request, Response};
use tokio::io::{AsyncRead, AsyncWrite};

use super::body::{RequestBody, ResponseBody};
use super::error::BodyError;
use super::tls::NoOpTlsAcceptorFactory;

/// HttpService Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub struct HttpServiceBuilder<St, F, B, AF, TlsSt> {
    factory: F,
    tls_factory: AF,
    _phantom: PhantomData<(St, B, TlsSt)>,
}

impl<St, B, E, F> HttpServiceBuilder<St, F, B, NoOpTlsAcceptorFactory, St>
where
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    /// Construct a new Service Builder with given service factory.
    pub fn new(factory: F) -> Self
    where
        F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>, Config = ()>,
        F::Service: 'static,

        St: AsyncRead + AsyncWrite + Unpin,
    {
        Self {
            factory,
            tls_factory: NoOpTlsAcceptorFactory,
            _phantom: PhantomData,
        }
    }

    #[cfg(feature = "http1")]
    /// Construct a new Http/1 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h1::RequestBody>` as Request type.
    /// This is a request type specific for Http/1 request body.
    pub fn h1(factory: F) -> super::h1::H1ServiceBuilder<F>
    where
        F: ServiceFactory<Request<super::h1::RequestBody>, Response = Response<ResponseBody<B>>, Config = ()>,
        F::Service: 'static,
    {
        super::h1::H1ServiceBuilder::new(factory)
    }

    #[cfg(feature = "http2")]
    /// Construct a new Http/2 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h2::RequestBody>` as Request type.
    /// This is a request type specific for Http/2 request body.
    pub fn h2(factory: F) -> super::h2::H2ServiceBuilder<St, F, NoOpTlsAcceptorFactory>
    where
        F: ServiceFactory<Request<super::h2::RequestBody>, Response = Response<ResponseBody<B>>, Config = ()>,
        F::Service: 'static,

        St: AsyncRead + AsyncWrite + Unpin,
    {
        super::h2::H2ServiceBuilder::new(factory)
    }

    #[cfg(feature = "http3")]
    /// Construct a new Http/3 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h3::RequestBody>` as Request type.
    /// This is a request type specific for Http/3 request body.
    pub fn h3(factory: F) -> super::h3::H3ServiceBuilder<F>
    where
        F: ServiceFactory<Request<super::h3::RequestBody>, Response = Response<ResponseBody<B>>, Config = ()>,
        F::Service: 'static,
    {
        super::h3::H3ServiceBuilder::new(factory)
    }
}
