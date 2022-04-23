use core::future::Future;

use crate::pipeline::{marker::MapErr, PipelineT};

use super::ServiceFactory;

impl<SF, Req, Arg, SF1, E> ServiceFactory<Req, Arg> for PipelineT<SF, SF1, MapErr>
where
    SF: ServiceFactory<Req, Arg>,

    SF1: Fn(SF::Error) -> E + Clone,
{
    type Response = SF::Response;
    type Error = E;
    type Service = PipelineT<SF::Service, SF1, MapErr>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.first.new_service(arg);
        let mapper = self.second.clone();
        async move {
            let service = service.await.map_err(&mapper)?;
            Ok(PipelineT::new(service, mapper))
        }
    }
}
