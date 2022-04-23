use core::future::Future;

use crate::pipeline::{marker::AndThen, PipelineT};

use super::ServiceFactory;

impl<SF, Req, SF1, Arg> ServiceFactory<Req, Arg> for PipelineT<SF, SF1, AndThen>
where
    SF: ServiceFactory<Req, Arg>,
    Arg: Clone,
    SF1: ServiceFactory<SF::Response, Arg, Error = SF::Error>,
{
    type Response = SF1::Response;
    type Error = SF1::Error;
    type Service = PipelineT<SF::Service, SF1::Service, AndThen>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.first.new_service(arg.clone());
        let then_service = self.second.new_service(arg);
        async move {
            let service = service.await?;
            let then_service = then_service.await?;
            Ok(PipelineT::new(service, then_service))
        }
    }
}
