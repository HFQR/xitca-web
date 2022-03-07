use core::future::Future;

use crate::factory::pipeline::marker;

use super::{pipeline::PipelineService, Service};

impl<S, Req, F, E> Service<Req> for PipelineService<S, F, marker::MapErr>
where
    S: Service<Req>,
    F: Fn(S::Error) -> E,
{
    type Response = S::Response;
    type Error = E;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move { self.service.call(req).await.map_err(&self.service2) }
    }
}
