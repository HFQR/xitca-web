use core::future::Future;

use crate::service::pipeline::PipelineService;

use super::{
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory,
};

impl<SF, Req, SF1, E> ServiceFactory<Req> for PipelineServiceFactory<SF, SF1, marker::MapErr>
where
    SF: ServiceFactory<Req>,
    SF1: Fn(SF::Error) -> E + Clone,
{
    type Response = SF::Response;
    type Error = E;

    type Config = SF::Config;
    type Service = PipelineService<SF::Service, SF1, marker::MapErr>;
    type InitError = SF::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: SF::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let mapper = self.factory2.clone();
        async move {
            let service = service.await?;
            Ok(PipelineService::new(service, mapper))
        }
    }
}
