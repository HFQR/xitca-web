use core::future::Future;

use crate::pipeline::{marker::Map, PipelineT};

use super::Service;

impl<S, Req, F, Res> Service<Req> for PipelineT<S, F, Map>
where
    S: Service<Req>,
    F: Fn(Result<S::Response, S::Error>) -> Result<Res, S::Error>,
{
    type Response = Res;
    type Error = S::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let res = self.first.call(req).await;
            (self.second)(res)
        }
    }
}
