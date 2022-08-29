use core::future::Future;

use crate::pipeline::{marker::AndThen, PipelineT};

use super::ReadyService;

impl<S, S1> ReadyService for PipelineT<S, S1, AndThen>
where
    S: ReadyService,
    S1: ReadyService,
{
    type Ready = PipelineT<S::Ready, S1::Ready>;
    type ReadyFuture<'f> = impl Future<Output = Self::Ready> where Self: 'f;

    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move {
            let first = self.first.ready().await;
            let second = self.second.ready().await;
            PipelineT::new(first, second)
        }
    }
}
