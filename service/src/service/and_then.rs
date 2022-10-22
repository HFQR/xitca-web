use core::future::Future;

use crate::pipeline::{
    marker::{AndThen, BuildAndThen},
    PipelineT,
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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s, 'f>(&'s self, arg: Arg) -> Self::Future<'f>
    where
        's: 'f,
        Arg: 'f,
    {
        async move {
            let first = self.first.call(arg.clone()).await?;
            let second = self.second.call(arg).await?;
            Ok(PipelineT::new(first, second))
        }
    }
}

impl<S, Req, S1> Service<Req> for PipelineT<S, S1, AndThen>
where
    S: Service<Req>,
    S1: Service<S::Response, Error = S::Error>,
{
    type Response = S1::Response;
    type Error = S::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s, 'f>(&'s self, req: Req) -> Self::Future<'f>
    where
        's: 'f,
        Req: 'f,
    {
        async {
            let res = self.first.call(req).await?;
            self.second.call(res).await
        }
    }
}
