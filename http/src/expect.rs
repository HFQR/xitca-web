//! Default expect handler. Pass through request unconditionally.

use std::{future::Future, marker::PhantomData};

use xitca_service::{Service, ServiceFactory};

pub struct ExpectHandler<F>(PhantomData<F>);

impl<F> Default for ExpectHandler<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F> ExpectHandler<F> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<F, Req> ServiceFactory<Req> for ExpectHandler<F>
where
    F: ServiceFactory<Req>,
{
    type Response = Req;
    type Error = F::Error;
    type Service = Self;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, _: ()) -> Self::Future {
        async { Ok(Self::new()) }
    }
}

impl<F, Req> Service<Req> for ExpectHandler<F>
where
    F: ServiceFactory<Req>,
{
    type Response = Req;
    type Error = F::Error;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, req: Req) -> Self::Future<'_> {
        async move { Ok(req) }
    }
}
