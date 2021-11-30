use core::future::Future;

use super::{
    pipeline::{marker::MapInitErr, PipelineServiceFactory},
    ServiceFactory,
};

impl<SF, Req, SF1, E> ServiceFactory<Req> for PipelineServiceFactory<SF, SF1, MapInitErr>
where
    SF: ServiceFactory<Req>,
    SF1: Fn(SF::InitError) -> E + Clone,
{
    type Response = SF::Response;
    type Error = SF::Error;
    type Config = SF::Config;
    type Service = SF::Service;
    type InitError = E;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: SF::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let mapper = self.factory2.clone();
        async move { service.await.map_err(mapper) }
    }
}
