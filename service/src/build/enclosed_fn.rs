use core::future::Future;

use crate::pipeline::{marker::EnclosedFn, PipelineT};

use super::BuildService;

impl<SF, Arg, T, Req> BuildService<Arg> for PipelineT<SF, T, EnclosedFn<Req>>
where
    SF: BuildService<Arg>,
    T: Clone,
{
    type Service = PipelineT<SF::Service, T, EnclosedFn<Req>>;
    type Error = SF::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.first.build(arg);
        let transform = self.second.clone();
        async move {
            let service = service.await?;
            Ok(PipelineT::new(service, transform))
        }
    }
}
