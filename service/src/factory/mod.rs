pub(crate) mod function;

mod and_then;
mod boxed;
mod enclosed;
mod enclosed_fn;
mod ext;
mod map;
mod map_err;

pub use self::{
    ext::ServiceFactoryExt,
    function::{fn_factory, fn_service},
};

use core::future::Future;

use alloc::{boxed::Box, rc::Rc, sync::Arc};

use crate::service::Service;

pub trait ServiceFactory<Req, Arg = ()> {
    /// Responses given by the created services.
    type Response;

    /// Errors produced by the created services.
    type Error;

    /// The kind of `Service` created by this factory.
    type Service: Service<Req, Response = Self::Response, Error = Self::Error>;

    /// The future of the `Service` instance.g
    type Future: Future<Output = Result<Self::Service, Self::Error>>;

    /// Create and return a new service asynchronously.
    fn new_service(&self, arg: Arg) -> Self::Future;
}

macro_rules! impl_alloc {
    ($alloc: ident) => {
        impl<F, Req, Arg> ServiceFactory<Req, Arg> for $alloc<F>
        where
            F: ServiceFactory<Req, Arg> + ?Sized,
        {
            type Response = F::Response;
            type Error = F::Error;
            type Service = F::Service;
            type Future = F::Future;

            fn new_service(&self, arg: Arg) -> Self::Future {
                (**self).new_service(arg)
            }
        }
    };
}

impl_alloc!(Box);
impl_alloc!(Rc);
impl_alloc!(Arc);
