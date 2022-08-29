use crate::pipeline::{marker::Map, PipelineT};

use super::ReadyService;

impl<S, F> ReadyService for PipelineT<S, F, Map>
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
