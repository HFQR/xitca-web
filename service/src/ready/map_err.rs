use crate::pipeline::{marker::MapErr, PipelineT};

use super::ReadyService;

impl<S, F> ReadyService for PipelineT<S, F, MapErr>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.first.ready().await
    }
}
