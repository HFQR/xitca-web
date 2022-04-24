use core::future::Future;

use crate::pipeline::{marker::AndThen, PipelineT};

use super::ReadyService;

impl<S, Req, S1> ReadyService<Req> for PipelineT<S, S1, AndThen>
where
    S: ReadyService<Req>,
    S1: ReadyService<S::Response, Error = S::Error>,
{
    type Ready = PipelineT<S::Ready, S1::Ready>;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move {
            let first = self.first.ready().await?;
            let second = self.second.ready().await?;
            Ok(PipelineT::new(first, second))
        }
    }
}
