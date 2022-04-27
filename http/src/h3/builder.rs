use std::{error, future::Future};

use futures_core::Stream;
use xitca_service::{BuildService, Service};

use crate::{body::ResponseBody, bytes::Bytes, error::BuildError, http::Response, request::Request};

use super::{body::RequestBody, service::H3Service};

type H3Request = Request<RequestBody>;

/// Http/3 Builder type.
/// Take in generic types of ServiceFactory for `quinn`.
pub struct H3ServiceBuilder<F> {
    factory: F,
}

impl<F, ResB, E> H3ServiceBuilder<F>
where
    F: BuildService,
    F::Service: Service<H3Request, Response = Response<ResponseBody<ResB>>> + 'static,

    ResB: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
{
    /// Construct a new Service Builder with given service factory.
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl<F, Arg> BuildService<Arg> for H3ServiceBuilder<F>
where
    F: BuildService<Arg>,
    F::Error: error::Error + 'static,
{
    type Service = H3Service<F::Service>;
    type Error = BuildError;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.factory.build(arg);
        async {
            let service = service.await?;
            Ok(H3Service::new(service))
        }
    }
}
