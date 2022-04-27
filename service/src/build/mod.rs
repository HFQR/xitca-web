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
    function::{fn_build, fn_service},
};

use core::future::Future;

use alloc::{boxed::Box, rc::Rc, sync::Arc};

pub trait BuildService<Arg = ()> {
    /// The kind of `Service` created by this factory.
    type Service;

    /// Error produced by creating service process.
    type Error;

    /// The future of the `Service` instance.g
    type Future: Future<Output = Result<Self::Service, Self::Error>>;

    /// Create and return a new service asynchronously.
    fn build(&self, arg: Arg) -> Self::Future;
}

macro_rules! impl_alloc {
    ($alloc: ident) => {
        impl<F, Arg> BuildService<Arg> for $alloc<F>
        where
            F: BuildService<Arg> + ?Sized,
        {
            type Service = F::Service;
            type Error = F::Error;
            type Future = F::Future;

            fn build(&self, arg: Arg) -> Self::Future {
                (**self).build(arg)
            }
        }
    };
}

impl_alloc!(Box);
impl_alloc!(Rc);
impl_alloc!(Arc);
