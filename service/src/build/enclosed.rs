use core::future::Future;

use crate::pipeline::{marker::Enclosed, PipelineT};

use super::BuildService;

impl<F, Arg, T> BuildService<Arg> for PipelineT<F, T, Enclosed>
where
    F: BuildService<Arg>,
    T: BuildService<F::Service> + Clone,
    F::Error: From<T::Error>,
{
    type Service = T::Service;
    type Error = F::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.first.build(arg);
        let middleware = self.second.clone();
        async move {
            let service = service.await?;
            let middleware = middleware.build(service).await?;
            Ok(middleware)
        }
    }
}
