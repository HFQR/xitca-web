use core::future::Future;

use crate::service::Service;

use super::ServiceFactory;

pub fn fn_service<F, Req, Fut, Res, Err>(f: F) -> FnServiceFactory<F>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    FnServiceFactory { f }
}

pub fn fn_factory<F, Arg, Fut, Svc, Err>(f: F) -> FnFactory<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    FnFactory { f }
}

#[derive(Clone)]
pub struct FnFactory<F> {
    f: F,
}

impl<F, Req, Arg, Fut, Svc, Res, Err> ServiceFactory<Req, Arg> for FnFactory<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
    Svc: Service<Req, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = Err;
    type Service = Svc;
    type Future = Fut;

    fn new_service(&self, arg: Arg) -> Self::Future {
        (self.f)(arg)
    }
}

#[derive(Clone)]
pub struct FnServiceFactory<F: Clone> {
    f: F,
}

impl<F, Req, Fut, Res, Err> ServiceFactory<Req> for FnServiceFactory<F>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Service = Self;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let f = self.f.clone();
        async { Ok(Self { f }) }
    }
}

impl<F, Req, Fut, Res, Err> Service<Req> for FnServiceFactory<F>
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
