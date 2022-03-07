use core::future::Future;

use crate::{factory::pipeline::marker, service::pipeline::PipelineService};

use super::ReadyService;

impl<S, Req, F, E> ReadyService<Req> for PipelineService<S, F, marker::MapErr>
where
    S: ReadyService<Req>,
    F: Fn(S::Error) -> E,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move {
            let first = self.service.ready().await.map_err(&self.service2)?;
            Ok(first)
        }
    }
}
