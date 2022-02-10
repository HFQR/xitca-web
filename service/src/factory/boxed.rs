use alloc::boxed::Box;

use crate::BoxFuture;

use super::ServiceFactory;

pub struct BoxedServiceFactory<F> {
    factory: F,
}

impl<F> BoxedServiceFactory<F> {
    pub(super) fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl<F, Req, Arg> ServiceFactory<Req, Arg> for BoxedServiceFactory<F>
where
    F: ServiceFactory<Req, Arg>,
    F::Future: 'static,
{
    type Response = F::Response;
    type Error = F::Error;
    type Service = F::Service;
    type Future = BoxFuture<'static, Self::Service, Self::Error>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        Box::pin(self.factory.new_service(arg))
    }
}
