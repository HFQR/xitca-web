use core::future::Future;

use crate::{factory::pipeline::marker, service::pipeline::PipelineService};

use super::ReadyService;

impl<S, Req, T, Fut, Res, Err> ReadyService<Req> for PipelineService<S, T, marker::TransformFn>
where
    S: ReadyService<Req> + Clone,
    T: Fn(S, Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S::Error>,
{
    type Ready = S::Ready;
    type ReadyFuture<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Ready, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move { self.service.ready().await.map_err(Into::into) }
    }
}
