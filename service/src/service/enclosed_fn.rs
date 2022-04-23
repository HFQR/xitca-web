use core::future::Future;

use crate::{
    async_closure::AsyncClosure,
    pipeline::{marker::EnclosedFn, PipelineT},
};

use super::Service;

impl<S, Req, T, Res, Err> Service<Req> for PipelineT<S, T, EnclosedFn>
where
    S: Service<Req>,
    T: for<'s> AsyncClosure<(&'s S, Req), Output = Result<Res, Err>>,
    Err: From<S::Error>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Res, Err>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        self.second.call((&self.first, req))
    }
}
