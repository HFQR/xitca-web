use core::{
    future::Future,
    task::{Context, Poll},
};

use super::ServiceFactory;
use crate::Service;

pub fn fn_service<F, Req, Fut, Res, Err>(f: F) -> FnServiceFactory<F>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    FnServiceFactory { f }
}

pub struct FnServiceFactory<F> {
    f: F,
}

impl<F, Req, Fut, Res, Err> ServiceFactory<Req> for FnServiceFactory<F>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
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
    type Future<'f> = Fut;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, req: Req) -> Self::Future<'_> {
        (self.f)(req)
    }
}
