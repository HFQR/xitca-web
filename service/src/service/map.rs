use core::future::Future;

use crate::pipeline::{
    marker::{BuildMap, Map},
    PipelineT,
};

use super::Service;

impl<SF, Arg, SF1> Service<Arg> for PipelineT<SF, SF1, BuildMap>
where
    SF: Service<Arg>,
    SF1: Clone,
{
    type Response = PipelineT<SF::Response, SF1, Map>;
    type Error = SF::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    fn call(&self, arg: Arg) -> Self::Future<'_> {
        let transform = self.second.clone();
        async move {
            let service = self.first.call(arg).await?;
            Ok(PipelineT::new(service, transform))
        }
    }
}

impl<S, Req, F, Res> Service<Req> for PipelineT<S, F, Map>
where
    S: Service<Req>,
    F: Fn(S::Response) -> Res,
{
    type Response = Res;
    type Error = S::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move { self.first.call(req).await.map(&self.second) }
    }
}
