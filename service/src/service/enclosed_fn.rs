use core::future::Future;

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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s>(&'s self, arg: Arg) -> Self::Future<'s>
    where
        Arg: 's,
    {
        async {
            let service = self.first.call(arg).await?;
            Ok(PipelineT::new(service, self.second.clone()))
        }
    }
}

impl<S, Req, T, Res, Err> Service<Req> for PipelineT<S, T, EnclosedFn>
where
    T: for<'s> AsyncClosure<(&'s S, Req), Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        self.second.call((&self.first, req))
    }
}
