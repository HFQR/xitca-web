use core::future::Future;

use crate::{async_closure::AsyncClosure, factory::pipeline::marker, service::pipeline::PipelineService};

use super::ReadyService;

impl<S, Req, T, Res, Err> ReadyService<Req> for PipelineService<S, T, marker::EnclosedFn>
where
    S: ReadyService<Req>,
    T: for<'s> AsyncClosure<(&'s S, Req), Output = Result<Res, Err>>,
    Err: From<S::Error>,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move { self.service.ready().await.map_err(Into::into) }
    }
}
