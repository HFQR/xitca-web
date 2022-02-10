use core::future::Future;

use crate::factory::pipeline::marker;

use super::{pipeline::PipelineService, Service};

impl<S, Req, S1> Service<Req> for PipelineService<S, S1, marker::Then>
where
    S: Service<Req>,
    S1: Service<Result<S::Response, S::Error>>,
{
    type Response = S1::Response;
    type Error = S1::Error;
    type Ready<'f>
    where
        Self: 'f,
    = S1::Ready<'f>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        // Only check service2's readiness.
        self.service2.ready()
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            // check service1's readiness in service call and pass potential error to service2.
            let res = ready_and_call(&self.service, req).await;
            self.service2.call(res).await
        }
    }
}

async fn ready_and_call<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
where
    S: Service<Req>,
{
    service.ready().await?;
    service.call(req).await
}
