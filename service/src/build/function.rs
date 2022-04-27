use core::{convert::Infallible, future::Future};

use crate::service::Service;

use super::BuildService;

pub fn fn_service<F, Req, Fut, Res, Err>(f: F) -> FnService<F>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    FnService { f }
}

pub fn fn_build<F, Arg, Fut, Svc, Err>(f: F) -> FnBuild<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    FnBuild { f }
}

#[derive(Clone)]
pub struct FnBuild<F> {
    f: F,
}

impl<F, Arg, Fut, Svc, Err> BuildService<Arg> for FnBuild<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    type Service = Svc;
    type Error = Err;
    type Future = Fut;

    fn build(&self, arg: Arg) -> Self::Future {
        (self.f)(arg)
    }
}

#[derive(Clone)]
pub struct FnService<F: Clone> {
    f: F,
}

impl<F> BuildService for FnService<F>
where
    F: Clone,
{
    type Service = Self;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, _: ()) -> Self::Future {
        let f = self.f.clone();
        async { Ok(Self { f }) }
    }
}

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
        (self.f)(req)
    }
}
