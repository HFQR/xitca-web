use crate::pipeline::{
    PipelineT,
    marker::{AndThen, BuildAndThen},
};

use super::Service;

impl<SF, Arg, SF1> Service<Arg> for PipelineT<SF, SF1, BuildAndThen>
where
    SF: Service<Arg>,
    Arg: Clone,
    SF1: Service<Arg>,
    SF1::Error: From<SF::Error>,
{
    type Response = PipelineT<SF::Response, SF1::Response, AndThen>;
    type Error = SF1::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        let first = self.first.call(arg.clone()).await?;
        let second = self.second.call(arg).await?;
        Ok(PipelineT::new(first, second))
    }
}

impl<S, Req, S1> Service<Req> for PipelineT<S, S1, AndThen>
where
    S: Service<Req>,
    S1: Service<S::Response, Error = S::Error>,
{
    type Response = S1::Response;
    type Error = S::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        let res = self.first.call(req).await?;
        self.second.call(res).await
    }
}
