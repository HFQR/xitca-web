use crate::pipeline::{marker::MapErr, PipelineT};

use super::ReadyService;

impl<S, F> ReadyService for PipelineT<S, F, MapErr>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type Future<'f> = S::Future<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::Future<'_> {
        self.first.ready()
    }
}
