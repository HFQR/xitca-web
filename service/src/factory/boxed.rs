use core::{future::Future, pin::Pin};

use alloc::boxed::Box;

use super::ServiceFactory;

pub struct BoxedServiceFactory<F> {
    factory: F
}

impl<F> BoxedServiceFactory<F> {
    pub(super) fn new(factory: F) -> Self {
        Self {
            factory
        }
    }
}

impl<F, Req> ServiceFactory<Req> for BoxedServiceFactory<F>
where
    F: ServiceFactory<Req>,
    F::Future: 'static
{
    type Response = F::Response;
    type Error = F::Error;
    type Config = F::Config;
    type Service = F::Service;
    type InitError = F::InitError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Service, Self::InitError>>>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        Box::pin(self.factory.new_service(cfg))
    }
}