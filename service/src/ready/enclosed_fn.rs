use crate::pipeline::{marker::EnclosedFn, PipelineT};

use super::ReadyService;

impl<S, T> ReadyService for PipelineT<S, T, EnclosedFn>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.first.ready().await
    }
}
