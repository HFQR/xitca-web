use core::future::Future;

use crate::factory::pipeline::marker;

use super::{pipeline::PipelineService, Service};

impl<S, Req, F, Res> Service<Req> for PipelineService<S, F, marker::Map>
where
    S: Service<Req>,
    F: Fn(Result<S::Response, S::Error>) -> Result<Res, S::Error>,
{
    type Response = Res;
    type Error = S::Error;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let res = self.service.call(req).await;
            (self.service2)(res)
        }
    }
}
