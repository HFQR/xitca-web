use crate::{ready::ReadyService, service::Service};

/// A middleware unconditionally treat inner service type as ready.
/// See [ReadyService] for detail.
#[derive(Clone, Copy)]
pub struct UncheckedReady;

impl<S, E> Service<Result<S, E>> for UncheckedReady {
    type Response = UncheckedReadyService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(UncheckedReadyService)
    }
}

pub struct UncheckedReadyService<S>(S);

impl<S, Req> Service<Req> for UncheckedReadyService<S>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.0.call(req).await
    }
}

impl<S> ReadyService for UncheckedReadyService<S> {
    type Ready = ();

    #[inline]
    async fn ready(&self) -> Self::Ready {}
}
