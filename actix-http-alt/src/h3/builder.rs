use std::future::Future;

use actix_server_alt::net::UdpStream;
use actix_service_alt::ServiceFactory;
use bytes::Bytes;
use futures_core::Stream;

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError};
use crate::request::HttpRequest;
use crate::response::{HttpResponse, ResponseError};

use super::body::RequestBody;
use super::service::H3Service;

/// Http/3 Builder type.
/// Take in generic types of ServiceFactory for `quinn`.
pub struct H3ServiceBuilder<F> {
    factory: F,
}

impl<F, B, E> H3ServiceBuilder<F>
where
    F: ServiceFactory<HttpRequest<RequestBody>, Response = HttpResponse<ResponseBody<B>>, Config = ()>,
    F::Service: 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    /// Construct a new Service Builder with given service factory.
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl<F, B, E> ServiceFactory<UdpStream> for H3ServiceBuilder<F>
where
    F: ServiceFactory<HttpRequest<RequestBody>, Response = HttpResponse<ResponseBody<B>>, Config = ()>,
    F::Service: 'static,

    F::Error: ResponseError<F::Response>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    type Response = ();
    type Error = HttpServiceError;
    type Config = ();
    type Service = H3Service<F::Service>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let service = self.factory.new_service(());
        async {
            let service = match service.await {
                Ok(service) => service,
                Err(_) => panic!("TODO"),
            };

            Ok(H3Service::new(service))
        }
    }
}
