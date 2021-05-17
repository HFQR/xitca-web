use std::marker::PhantomData;

use actix_service_alt::ServiceFactory;
use bytes::Bytes;
use futures_core::Stream;
use tokio::io::{AsyncRead, AsyncWrite};

use super::body::ResponseBody;
use super::error::BodyError;
use super::request::HttpRequest;
use super::response::HttpResponse;
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
        F: ServiceFactory<HttpRequest, Response = HttpResponse<ResponseBody<B>>, Config = ()>,
        F::Service: 'static,

        St: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        Self {
            factory,
            tls_factory: NoOpTlsAcceptorFactory,
            _phantom: PhantomData,
        }
    }

    #[cfg(feature = "http2")]
    /// Construct a new Http/2 ServiceBuilder.
    ///
    /// Note factory type F ues `HttpRequest<h2::RequestBody>` as Request type.
    /// This is a request type specific for Http/2 request body.
    pub fn h2(factory: F) -> super::h2::H2ServiceBuilder<St, F, NoOpTlsAcceptorFactory>
    where
        F: ServiceFactory<
            HttpRequest<super::h2::RequestBody>,
            Response = HttpResponse<ResponseBody<B>>,
            Config = (),
        >,
        F::Service: 'static,

        St: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        super::h2::H2ServiceBuilder::new(factory)
    }

    #[cfg(feature = "http3")]
    /// Construct a new Http/3 ServiceBuilder.
    ///
    /// Note factory type F ues `HttpRequest<h3::RequestBody>` as Request type.
    /// This is a request type specific for Http/3 request body.
    pub fn h3(factory: F) -> super::h3::H3ServiceBuilder<F>
    where
        F: ServiceFactory<
            HttpRequest<super::h3::RequestBody>,
            Response = HttpResponse<ResponseBody<B>>,
            Config = (),
        >,
        F::Service: 'static,
    {
        super::h3::H3ServiceBuilder::new(factory)
    }
}
