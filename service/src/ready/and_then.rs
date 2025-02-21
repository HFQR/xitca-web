use crate::pipeline::{PipelineT, marker::AndThen};

use super::ReadyService;

impl<S, S1> ReadyService for PipelineT<S, S1, AndThen>
where
    S: ReadyService,
    S1: ReadyService,
{
    type Ready = PipelineT<S::Ready, S1::Ready>;

    async fn ready(&self) -> Self::Ready {
        let first = self.first.ready().await;
        let second = self.second.ready().await;
        PipelineT::new(first, second)
    }
}
