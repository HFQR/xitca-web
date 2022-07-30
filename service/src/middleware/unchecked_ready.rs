use core::{convert::Infallible, future::Future};

use crate::{build::BuildService, ready::ReadyService, service::Service};

/// A middleware unconditionally treat inner service type as ready.
/// See [ReadyService] for detail.
#[derive(Clone, Copy)]
pub struct UncheckedReady;

impl<S> BuildService<S> for UncheckedReady {
    type Service = UncheckedReadyService<S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, service: S) -> Self::Future {
        async { Ok(UncheckedReadyService { service }) }
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
    type Future<'f> = S::Future<'f>
    where
        S: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        self.service.call(req)
    }
}

impl<S, Req> ReadyService<Req> for UncheckedReadyService<S>
where
    S: Service<Req>,
{
    type Ready = ();

    type ReadyFuture<'f> = impl Future<Output = Self::Ready>
    where
        S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async {}
    }
}
