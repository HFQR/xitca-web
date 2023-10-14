use crate::{
    async_closure::AsyncClosure,
    pipeline::{
        marker::{BuildEnclosedFn, EnclosedFn},
        PipelineT,
    },
};

use super::Service;

impl<SF, Arg, T> Service<Arg> for PipelineT<SF, T, BuildEnclosedFn>
where
    SF: Service<Arg>,
    T: Clone,
{
    type Response = PipelineT<SF::Response, T, EnclosedFn>;
    type Error = SF::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        let service = self.first.call(arg).await?;
        Ok(PipelineT::new(service, self.second.clone()))
    }
}

impl<S, Req, T, Res, Err> Service<Req> for PipelineT<S, T, EnclosedFn>
where
    T: for<'s> AsyncClosure<(&'s S, Req), Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;

    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.second.call((&self.first, req)).await
    }
}
