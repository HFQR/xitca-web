use crate::pipeline::{
    PipelineT,
    marker::{BuildMapErr, MapErr},
};

use super::Service;

impl<SF, Arg, SF1> Service<Arg> for PipelineT<SF, SF1, BuildMapErr>
where
    SF: Service<Arg>,
    SF1: Clone,
{
    type Response = PipelineT<SF::Response, SF1, MapErr>;
    type Error = SF::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        let service = self.first.call(arg).await?;
        Ok(PipelineT::new(service, self.second.clone()))
    }
}

impl<S, Req, F, Err> Service<Req> for PipelineT<S, F, MapErr>
where
    S: Service<Req>,
    F: Fn(S::Error) -> Err,
{
    type Response = S::Response;
    type Error = Err;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.first.call(req).await.map_err(&self.second)
    }
}
