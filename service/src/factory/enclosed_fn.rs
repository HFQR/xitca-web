use core::future::Future;

use crate::{async_closure::AsyncClosure, service::pipeline::PipelineService};

use super::{
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory,
};

impl<SF, Req, Arg, T, Fut, Res, Err> ServiceFactory<Req, Arg> for PipelineServiceFactory<SF, T, marker::EnclosedFn>
where
    SF: ServiceFactory<Req, Arg>,
    SF::Service: Clone,

    T: Fn(SF::Service, Req) -> Fut + Clone,

    Fut: Future<Output = Result<Res, Err>>,

    Err: From<SF::Error>,
{
    type Response = Res;
    type Error = Err;
    type Service = PipelineService<SF::Service, T, marker::EnclosedFn>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.factory.new_service(arg);
        let transform = self.factory2.clone();

        async move {
            let service = service.await?;
            Ok(PipelineService::new(service, transform))
        }
    }
}

impl<SF, Req, Arg, T, Res, Err> ServiceFactory<Req, Arg> for PipelineServiceFactory<SF, T, marker::EnclosedFn2>
where
    SF: ServiceFactory<Req, Arg>,
    T: for<'s> AsyncClosure<(&'s SF::Service, Req), Output = Result<Res, Err>> + Clone,
    Err: From<SF::Error>,
{
    type Response = Res;
    type Error = Err;
    type Service = PipelineService<SF::Service, T, marker::EnclosedFn2>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.factory.new_service(arg);
        let transform = self.factory2.clone();

        async move {
            let service = service.await?;
            Ok(PipelineService::new(service, transform))
        }
    }
}
