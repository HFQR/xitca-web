use core::future::Future;

use crate::pipeline::{marker::MapErr, PipelineT};

use super::BuildService;

impl<SF, Arg, SF1> BuildService<Arg> for PipelineT<SF, SF1, MapErr>
where
    SF: BuildService<Arg>,
    SF1: Clone,
{
    type Service = PipelineT<SF::Service, SF1, MapErr>;
    type Error = SF::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.first.build(arg);
        let mapper = self.second.clone();
        async move {
            let service = service.await?;
            Ok(PipelineT::new(service, mapper))
        }
    }
}
