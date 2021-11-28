use core::future::Future;

use crate::service::pipeline::PipelineService;

use super::{
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory,
};

impl<SF, Req, SF1, Res> ServiceFactory<Req> for PipelineServiceFactory<SF, SF1, marker::Map>
where
    SF: ServiceFactory<Req>,
    SF1: Fn(Result<SF::Response, SF::Error>) -> Result<Res, SF::Error> + Clone,
{
    type Response = Res;
    type Error = SF::Error;
    type Config = SF::Config;
    type Service = PipelineService<SF::Service, SF1, marker::Map>;
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
