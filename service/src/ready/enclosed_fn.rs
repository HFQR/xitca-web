use crate::pipeline::{marker::EnclosedFn, PipelineT};

use super::ReadyService;

impl<S, T> ReadyService for PipelineT<S, T, EnclosedFn>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.first.ready()
    }
}
