pub(crate) mod function;

mod and_then;
#[cfg(feature = "alloc")]
mod boxed;
mod enclosed;
mod enclosed_fn;
mod ext;
mod map;
mod map_err;
mod opt;

pub use self::{
    ext::BuildServiceExt,
    function::{fn_build, fn_service},
};

use core::future::Future;

/// Trait for simulate `Fn<(&Self, Arg)> -> impl Future<Output = Result<(impl Service<Req> + !Send), E>> + Send`.
/// This trait is used to construct a `!Send` type from a `Send` type.
/// (From xitca's point of view only)
pub trait BuildService<Arg = ()> {
    /// The Ok part of output future.
    type Service;

    /// The Err part of output future
    type Error;

    /// The output future.
    type Future: Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future;
}

#[cfg(feature = "alloc")]
mod alloc_impl {
    use super::BuildService;

    use alloc::{boxed::Box, rc::Rc, sync::Arc};

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
}
