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
    type Ready<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        async move { self.service.ready().await.map_err(&self.service2) }
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move { self.service.call(req).await.map_err(&self.service2) }
    }
}
