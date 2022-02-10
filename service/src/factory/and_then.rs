use core::future::Future;

use crate::service::pipeline::PipelineService;

use super::{
    pipeline::{marker::AndThen, PipelineServiceFactory},
    ServiceFactory,
};

impl<SF, Req, SF1, Arg> ServiceFactory<Req, Arg> for PipelineServiceFactory<SF, SF1, AndThen>
where
    SF: ServiceFactory<Req, Arg>,

    Arg: Clone,

    SF1: ServiceFactory<SF::Response, Arg>,
    SF1::Error: From<SF::Error>,
{
    type Response = SF1::Response;
    type Error = SF1::Error;
    type Service = PipelineService<SF::Service, SF1::Service, AndThen>;
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
