use crate::pipeline::{marker::Map, PipelineT};

use super::ReadyService;

impl<S, Req, F, Res> ReadyService<Req> for PipelineT<S, F, Map>
where
    S: ReadyService<Req>,
    F: Fn(Result<S::Response, S::Error>) -> Result<Res, S::Error>,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.first.ready()
    }
}
