use crate::pipeline::{marker::MapErr, PipelineT};

use super::ReadyService;

impl<S, Req, F, E> ReadyService<Req> for PipelineT<S, F, MapErr>
where
    S: ReadyService<Req>,
    F: Fn(S::Error) -> E,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.first.ready()
    }
}
