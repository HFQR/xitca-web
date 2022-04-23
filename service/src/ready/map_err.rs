use core::future::Future;

use crate::pipeline::{marker::MapErr, PipelineT};

use super::ReadyService;

impl<S, Req, F, E> ReadyService<Req> for PipelineT<S, F, MapErr>
where
    S: ReadyService<Req>,
    F: Fn(S::Error) -> E,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move {
            let first = self.first.ready().await.map_err(&self.second)?;
            Ok(first)
        }
    }
}
