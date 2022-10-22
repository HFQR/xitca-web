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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s, 'f>(&'s self, arg: Arg) -> Self::Future<'f>
    where
        's: 'f,
        Arg: 'f,
    {
        async {
            let service = self.first.call(arg).await?;
            Ok(PipelineT::new(service, self.second.clone()))
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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s, 'f>(&'s self, req: Req) -> Self::Future<'f>
    where
        's: 'f,
        Req: 'f,
    {
        async { self.first.call(req).await.map(&self.second) }
    }
}
