use core::future::Future;

use crate::pipeline::{marker::AndThen, PipelineT};

use super::Service;

impl<S, Req, S1> Service<Req> for PipelineT<S, S1, AndThen>
where
    S: Service<Req>,
    S1: Service<S::Response, Error = S::Error>,
{
    type Response = S1::Response;
    type Error = S1::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let res = self.first.call(req).await?;
            self.second.call(res).await
        }
    }
}
