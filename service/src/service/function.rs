use core::{
    convert::Infallible,
    future::{ready, Future, Ready},
};

use super::Service;

/// Shortcut for transform a given Fn into type impl [Service] trait.
pub fn fn_service<F, Req, Fut, Res, Err>(f: F) -> FnBuild<impl Fn(()) -> Ready<Result<FnService<F>, Infallible>>>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn_build(move |_| ready(Ok(FnService(f.clone()))))
}

/// Shortcut for transform a given Fn into type impl [Service] trait.
pub fn fn_build<F, Arg, Fut, Svc, Err>(f: F) -> FnBuild<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    FnBuild(f)
}

#[derive(Clone)]
pub struct FnBuild<F>(F);

impl<F, Arg, Fut, Svc, Err> Service<Arg> for FnBuild<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    type Response = Svc;
    type Error = Err;
    type Future<'f> = Fut where Self: 'f;

    fn call(&self, arg: Arg) -> Self::Future<'_> {
        (self.0)(arg)
    }
}

#[derive(Clone)]
pub struct FnService<F>(F);

impl<F, Req, Fut, Res, Err> Service<Req> for FnService<F>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = Fut where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        (self.0)(req)
    }
}
