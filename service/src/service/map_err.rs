use core::future::Future;

use crate::pipeline::{
    marker::{BuildMapErr, MapErr},
    PipelineT,
};

use super::Service;

impl<SF, Arg, SF1> Service<Arg> for PipelineT<SF, SF1, BuildMapErr>
where
    SF: Service<Arg>,
    SF1: Clone,
{
    type Response = PipelineT<SF::Response, SF1, MapErr>;
    type Error = SF::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    fn call(&self, arg: Arg) -> Self::Future<'_> {
        async move {
            let service = self.first.call(arg).await?;
            Ok(PipelineT::new(service, self.second.clone()))
        }
    }
}

impl<S, Req, F, Err> Service<Req> for PipelineT<S, F, MapErr>
where
    S: Service<Req>,
    F: Fn(S::Error) -> Err,
{
    type Response = S::Response;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move { self.first.call(req).await.map_err(&self.second) }
    }
}
