use core::future::Future;

use crate::service::pipeline::PipelineService;

use super::{
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory,
};

// TODO: temporary public type alias that should be removed in the future.
pub type MapErrorServiceFactory<SF, SF1> = PipelineServiceFactory<SF, SF1, marker::MapErr>;

impl<SF, Req, Arg, SF1, E> ServiceFactory<Req, Arg> for PipelineServiceFactory<SF, SF1, marker::MapErr>
where
    SF: ServiceFactory<Req, Arg>,

    SF1: Fn(SF::Error) -> E + Clone,
{
    type Response = SF::Response;
    type Error = E;
    type Service = PipelineService<SF::Service, SF1, marker::MapErr>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.factory.new_service(arg);
        let mapper = self.factory2.clone();
        async move {
            let service = service.await.map_err(&mapper)?;

            Ok(PipelineService::new(service, mapper))
        }
    }
}
