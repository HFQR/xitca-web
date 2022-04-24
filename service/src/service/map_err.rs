use core::future::Future;

use crate::pipeline::{marker::MapErr, PipelineT};

use super::Service;

impl<S, Req, F, E> Service<Req> for PipelineT<S, F, MapErr>
where
    S: Service<Req>,
    F: Fn(S::Error) -> E,
{
    type Response = S::Response;
    type Error = E;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move { self.first.call(req).await.map_err(&self.second) }
    }
}
