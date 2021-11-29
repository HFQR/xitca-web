pub(crate) mod pipeline;

mod and_then;
mod ext;
mod function;
mod map;
mod map_err;
mod object;
mod then;

pub use self::{ext::ServiceFactoryExt, function::fn_service, object::ServiceFactoryObject};

use core::future::Future;

use alloc::{boxed::Box, rc::Rc, sync::Arc};

use crate::service::Service;

pub trait ServiceFactory<Req> {
    /// Responses given by the created services.
    type Response;

    /// Errors produced by the created services.
    type Error;

    /// Service factory configuration.
    type Config;

    /// The kind of `Service` created by this factory.
    type Service: Service<Req, Response = Self::Response, Error = Self::Error>;

    /// Errors potentially raised while building a service.
    type InitError;

    /// The future of the `Service` instance.g
    type Future: Future<Output = Result<Self::Service, Self::InitError>>;

    /// Create and return a new service asynchronously.
    fn new_service(&self, cfg: Self::Config) -> Self::Future;
}

macro_rules! impl_alloc {
    ($alloc: ident) => {
        impl<F, Req> ServiceFactory<Req> for $alloc<F>
        where
            F: ServiceFactory<Req> + ?Sized,
        {
            type Response = F::Response;
            type Error = F::Error;
            type Config = F::Config;
            type Service = F::Service;
            type InitError = F::InitError;
            type Future = F::Future;

            fn new_service(&self, cfg: Self::Config) -> Self::Future {
                (**self).new_service(cfg)
            }
        }
    };
}

impl_alloc!(Box);
impl_alloc!(Rc);
impl_alloc!(Arc);
