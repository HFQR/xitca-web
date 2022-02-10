use core::future::Future;

use crate::service::pipeline::PipelineService;

use super::{
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory,
};

impl<SF, Req, Arg, T, Fut, Res, Err> ServiceFactory<Req, Arg> for PipelineServiceFactory<SF, T, marker::TransformFn>
where
    SF: ServiceFactory<Req, Arg>,
    SF::Service: Clone,

    T: Fn(SF::Service, Req) -> Fut + Clone,

    Fut: Future<Output = Result<Res, Err>>,

    Err: From<SF::Error>,
{
    type Response = Res;
    type Error = Err;
    type Service = PipelineService<SF::Service, T, marker::TransformFn>;
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
