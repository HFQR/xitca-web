use crate::pipeline::{
    PipelineT,
    marker::{BuildMap, Map},
};

use super::Service;

impl<SF, Arg, SF1> Service<Arg> for PipelineT<SF, SF1, BuildMap>
where
    SF: Service<Arg>,
    SF1: Clone,
{
    type Response = PipelineT<SF::Response, SF1, Map>;
    type Error = SF::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        let service = self.first.call(arg).await?;
        Ok(PipelineT::new(service, self.second.clone()))
    }
}

impl<S, Req, F, Res> Service<Req> for PipelineT<S, F, Map>
where
    S: Service<Req>,
    F: Fn(S::Response) -> Res,
{
    type Response = Res;
    type Error = S::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.first.call(req).await.map(&self.second)
    }
}
