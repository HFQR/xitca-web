use core::future::Future;

use crate::factory::pipeline::marker;

use super::{pipeline::PipelineService, Service};

impl<S, Req, T, Fut, Res, Err> Service<Req> for PipelineService<S, T, marker::TransformFn>
where
    S: Service<Req> + Clone,
    T: Fn(S, Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S::Error>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f>
    where
        Self: 'f,
    = Fut;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        (self.service2)(self.service.clone(), req)
    }
}
