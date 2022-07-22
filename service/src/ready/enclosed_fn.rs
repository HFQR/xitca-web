use crate::{
    async_closure::AsyncClosure,
    pipeline::{marker::EnclosedFn, PipelineT},
};

use super::ReadyService;

impl<S, Req, Req2, T, Res, Err> ReadyService<Req> for PipelineT<S, T, EnclosedFn<Req2>>
where
    S: ReadyService<Req2>,
    T: for<'s> AsyncClosure<(&'s S, Req), Output = Result<Res, Err>>,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.first.ready()
    }
}
