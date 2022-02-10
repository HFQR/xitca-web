use core::future::Future;

use crate::service::pipeline::PipelineService;

use super::{
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory,
};

impl<SF, Req, Arg, SF1> ServiceFactory<Req, Arg> for PipelineServiceFactory<SF, SF1, marker::Then>
where
    SF: ServiceFactory<Req, Arg>,

    Arg: Clone,

    SF1::Error: From<SF::Error>,
    SF1: ServiceFactory<Result<SF::Response, SF::Error>, Arg>,
{
    type Response = SF1::Response;
    type Error = SF1::Error;
    type Service = PipelineService<SF::Service, SF1::Service, marker::Then>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.factory.new_service(arg.clone());
        let then_service = self.factory2.new_service(arg);

        async move {
            let service = service.await?;
            let then_service = then_service.await?;
            Ok(PipelineService::new(service, then_service))
        }
    }
}
