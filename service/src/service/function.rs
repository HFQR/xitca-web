#![allow(clippy::redundant_async_block)]

use core::{
    convert::Infallible,
    future::{ready, Future, Ready},
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
pub fn fn_service<F, Req, Fut, Res, Err>(f: F) -> FnService<impl Fn(()) -> Ready<Result<FnService<F>, Infallible>>>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn_build(move |_| ready(Ok(FnService(f.clone()))))
}

#[derive(Clone)]
pub struct FnService<F>(F);

impl<F, Req, Fut, Res, Err> Service<Req> for FnService<F>
where
    F: Fn(Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        async { (self.0)(req).await }
    }
}
