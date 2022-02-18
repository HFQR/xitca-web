use crate::{factory::pipeline::marker, service::pipeline::PipelineService};

use super::ReadyService;

impl<S, Req, F, Res> ReadyService<Req> for PipelineService<S, F, marker::Map>
where
    S: ReadyService<Req>,
    F: Fn(Result<S::Response, S::Error>) -> Result<Res, S::Error>,
{
    type Ready = S::Ready;
    type ReadyFuture<'f>
    where
        Self: 'f,
    = S::ReadyFuture<'f>;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}
