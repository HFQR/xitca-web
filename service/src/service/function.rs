use core::{convert::Infallible, future::Future};

use super::Service;

/// Shortcut for transform a given Fn into type impl [Service] trait.
pub fn fn_build<F>(f: F) -> FnService<F> {
    FnService(f)
}

/// Shortcut for transform a given Fn into type impl [Service] trait.
pub fn fn_service<F>(f: F) -> FnService<impl AsyncFn(()) -> Result<FnService<F>, Infallible> + Clone>
where
    F: Clone,
{
    fn_build(async move |_| Ok(FnService(f.clone())))
}

/// new type for implementing [Service] trait to a `Fn(Req) -> impl Future<Output = Result<Res, Err>>` type.
#[derive(Clone)]
pub struct FnService<F>(F);

impl<F, Req, Res, Err> Service<Req> for FnService<F>
where
    F: AsyncFn(Req) -> Result<Res, Err>,
{
    type Response = Res;
    type Error = Err;

    #[inline]
    fn call(&self, req: Req) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        (self.0)(req)
    }
}
