use core::future::Future;

use crate::pipeline::{marker::Map, PipelineT};

use super::ServiceFactory;

impl<SF, Req, Arg, SF1, Res> ServiceFactory<Req, Arg> for PipelineT<SF, SF1, Map>
where
    SF: ServiceFactory<Req, Arg>,
    SF1: Fn(SF::Response) -> Res + Clone,
{
    type Response = Res;
    type Error = SF::Error;
    type Service = PipelineT<SF::Service, SF1, Map>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.first.new_service(arg);
        let mapper = self.second.clone();
        async move {
            let service = service.await?;
            Ok(PipelineT::new(service, mapper))
        }
    }
}
