use core::future::Future;

use crate::{factory::pipeline::marker, service::pipeline::PipelineService};

use super::{pipeline::PipelineReady, ReadyService};

impl<S, Req, S1> ReadyService<Req> for PipelineService<S, S1, marker::AndThen>
where
    S: ReadyService<Req>,
    S1: ReadyService<S::Response>,
    S1::Error: From<S::Error>,
{
    type Ready = PipelineReady<S::Ready, S1::Ready>;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move {
            let first = self.service.ready().await?;
            let second = self.service2.ready().await?;
            Ok(PipelineReady::new(first, second))
        }
    }
}
