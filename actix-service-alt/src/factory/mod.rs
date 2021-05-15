use core::future::Future;

use crate::service::Service;

pub trait ServiceFactory<Req> {
    /// Responses given by the created services.
    type Response;

    /// Errors produced by the created services.
    type Error;

    /// Service factory configuration.
    type Config;

    /// The kind of `Service` created by this factory.
    #[rustfmt::skip]
    type Service: for<'r> Service<Request<'r> = Req, Response = Self::Response, Error = Self::Error>;

    /// Errors potentially raised while building a service.
    type InitError;

    /// The future of the `Service` instance.g
    type Future: Future<Output = Result<Self::Service, Self::InitError>>;

    /// Create and return a new service asynchronously.
    fn new_service(&self, cfg: Self::Config) -> Self::Future;
}
