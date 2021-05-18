use std::{
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use actix_service_alt::{Service, ServiceFactory};

pub struct ExpectHandler<Req, Err>(PhantomData<(Req, Err)>);

impl<Req, Err> ServiceFactory<Req> for ExpectHandler<Req, Err>
where
    Req: 'static,
    Err: 'static,
{
    type Response = Req;
    type Error = Err;
    type Config = ();
    type Service = ExpectHandler<Req, Err>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async { Ok(ExpectHandler(PhantomData)) }
    }
}

impl<Req, Err> Service<Req> for ExpectHandler<Req, Err>
where
    Req: 'static,
    Err: 'static,
{
    type Response = Req;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call<'c>(&'c self, req: Req) -> Self::Future<'c>
    where
        Req: 'c,
    {
        async { Ok(req) }
    }
}
