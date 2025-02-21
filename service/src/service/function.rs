use core::{
    convert::Infallible,
    future::{Future, Ready, ready},
};

use super::Service;

/// Shortcut for transform a given Fn into type impl [Service] trait.
pub fn fn_build<F, Arg, Fut, Svc, Err>(f: F) -> FnService<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    FnService(f)
}

/// Shortcut for transform a given Fn into type impl [Service] trait.
pub fn fn_service<F, Req, Fut, Res, Err>(
    f: F,
) -> FnService<impl Fn(()) -> Ready<Result<FnService<F>, Infallible>> + Clone>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn_build(move |_| ready(Ok(FnService(f.clone()))))
}

/// new type for implementing [Service] trait to a `Fn(Req) -> impl Future<Output = Result<Res, Err>>` type.
#[derive(Clone)]
pub struct FnService<F>(F);

impl<F, Req, Fut, Res, Err> Service<Req> for FnService<F>
where
    F: Fn(Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;

    #[inline]
    fn call(&self, req: Req) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        (self.0)(req)
    }
}
