use core::future::Future;

use crate::pipeline::{marker::AndThen, PipelineT};

use super::BuildService;

impl<SF, SF1, Arg> BuildService<Arg> for PipelineT<SF, SF1, AndThen>
where
    SF: BuildService<Arg>,
    Arg: Clone,
    SF1: BuildService<Arg>,
    SF1::Error: From<SF::Error>,
{
    type Service = PipelineT<SF::Service, SF1::Service, AndThen>;
    type Error = SF1::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let first = self.first.build(arg.clone());
        let second = self.second.build(arg);
        async move {
            let first = first.await?;
            let second = second.await?;
            Ok(PipelineT::new(first, second))
        }
    }
}
