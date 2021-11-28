use core::future::Future;

use crate::{
    factory::{
        pipeline::{marker, PipelineServiceFactory},
        ServiceFactory,
    },
    service::{pipeline::PipelineService, Service},
};

impl<SF, Req, T, Fut, Res, Err> ServiceFactory<Req> for PipelineServiceFactory<SF, T, marker::TransformFn>
where
    SF: ServiceFactory<Req>,
    T: for<'s> Fn(&'s SF::Service, Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<SF::Error>,
{
    type Response = Res;
    type Error = Err;
    type Config = SF::Config;
    type Service = PipelineService<SF::Service, T, marker::TransformFn>;
    type InitError = SF::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let transform = self.factory2.clone();

        async move {
            let service = service.await?;
            Ok(PipelineService::new(service, transform))
        }
    }
}

impl<S, Req, T, Fut, Res, Err> Service<Req> for PipelineService<S, T, marker::TransformFn>
where
    S: Service<Req>,
    T: for<'s> Fn(&'s S, Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S::Error>,
{
    type Response = Res;
    type Error = Err;
    type Ready<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = Fut;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        async move { Ok(self.service.ready().await?) }
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        (self.service2)(&self.service, req)
    }
}
