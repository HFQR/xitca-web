use core::future::Future;

use crate::service::pipeline::PipelineService;

use super::{
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory,
};

impl<SF, Req, SF1> ServiceFactory<Req> for PipelineServiceFactory<SF, SF1, marker::Then>
where
    SF: ServiceFactory<Req>,
    SF::InitError: From<SF1::InitError>,
    SF::Config: Clone,

    SF1: ServiceFactory<Result<SF::Response, SF::Error>, Config = SF::Config>,
{
    type Response = SF1::Response;
    type Error = SF1::Error;
    type Config = SF::Config;
    type Service = PipelineService<SF::Service, SF1::Service, marker::Then>;
    type InitError = SF::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: SF::Config) -> Self::Future {
        let service = self.factory.new_service(cfg.clone());
        let then_service = self.factory2.new_service(cfg);

        async move {
            let service = service.await?;
            let then_service = then_service.await?;
            Ok(PipelineService::new(service, then_service))
        }
    }
}
