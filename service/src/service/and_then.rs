use core::future::Future;

use crate::factory::pipeline::marker::AndThen;

use super::{pipeline::PipelineService, Service};

impl<S, Req, S1> Service<Req> for PipelineService<S, S1, AndThen>
where
    S: Service<Req>,
    S1: Service<S::Response>,
    S1::Error: From<S::Error>,
{
    type Response = S1::Response;
    type Error = S1::Error;
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
        async move {
            self.service.ready().await?;
            self.service2.ready().await
        }
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let res = self.service.call(req).await?;
            self.service2.call(res).await
        }
    }
}
