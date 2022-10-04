use core::future::Future;

use crate::pipeline::{marker::Enclosed, PipelineE, PipelineT};

use super::BuildService;

impl<F, Arg, T> BuildService<Arg> for PipelineT<F, T, Enclosed>
where
    F: BuildService<Arg>,
    T: BuildService<F::Service> + Clone,
{
    type Service = T::Service;
    type Error = PipelineE<F::Error, T::Error>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.first.build(arg);
        let middleware = self.second.clone();
        async move {
            let service = service.await.map_err(PipelineE::First)?;
            middleware.build(service).await.map_err(PipelineE::Second)
        }
    }
}
