use crate::pipeline::{marker::Map, PipelineT};

use super::ReadyService;

impl<S, Req, F, Res> ReadyService<Req> for PipelineT<S, F, Map>
where
    S: ReadyService<Req>,
    F: Fn(S::Response) -> Res,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.first.ready()
    }
}
