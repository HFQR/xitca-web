use core::convert::Infallible;

use crate::{ready::ReadyService, service::Service};

/// A middleware unconditionally treat inner service type as ready.
/// See [ReadyService] for detail.
#[derive(Clone, Copy)]
pub struct UncheckedReady;

impl<S> Service<S> for UncheckedReady {
    type Response = UncheckedReadyService<S>;
    type Error = Infallible;

    async fn call(&self, service: S) -> Result<Self::Response, Self::Error> {
        Ok(UncheckedReadyService { service })
    }
}

pub struct UncheckedReadyService<S> {
    service: S,
}

impl<S, Req> Service<Req> for UncheckedReadyService<S>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.service.call(req).await
    }
}

impl<S> ReadyService for UncheckedReadyService<S> {
    type Ready = ();

    #[inline]
    async fn ready(&self) -> Self::Ready {}
}
