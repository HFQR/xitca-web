use crate::{
    async_fn,
    pipeline::{marker, PipelineT},
    service::Service,
};

/// transform given async function to middleware that can be implied to certain service
/// through [ServiceExt::enclosed] API.
///
/// [ServiceExt::enclosed]: crate::service::ServiceExt::enclosed
pub struct AsyncFn<F>(pub F);

impl<S, E, F> Service<Result<S, E>> for AsyncFn<F>
where
    F: Clone,
{
    type Response = PipelineT<S, F, marker::AsyncFn>;
    type Error = E;

    async fn call(&self, arg: Result<S, E>) -> Result<Self::Response, Self::Error> {
        arg.map(|service| PipelineT::new(service, self.0.clone()))
    }
}

impl<S, Req, F, Res, Err> Service<Req> for PipelineT<S, F, marker::AsyncFn>
where
    F: for<'s> async_fn::AsyncFn<(&'s S, Req), Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.second.call((&self.first, req)).await
    }
}
