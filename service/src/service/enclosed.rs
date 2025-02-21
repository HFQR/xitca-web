use crate::pipeline::{PipelineT, marker::BuildEnclosed};

use super::Service;

impl<F, T, Arg> Service<Arg> for PipelineT<F, T, BuildEnclosed>
where
    F: Service<Arg>,
    T: Service<Result<F::Response, F::Error>>,
{
    type Response = T::Response;
    type Error = T::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        let res = self.first.call(arg).await;
        self.second.call(res).await
    }
}
