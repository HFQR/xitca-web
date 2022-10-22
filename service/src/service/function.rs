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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s, 'f>(&'s self, arg: Arg) -> Self::Future<'f>
    where
        's: 'f,
        Arg: 'f,
    {
        async { (self.0)(arg).await }
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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s, 'f>(&'s self, req: Req) -> Self::Future<'f>
    where
        's: 'f,
        Req: 'f,
    {
        async { (self.0)(req).await }
    }
}
