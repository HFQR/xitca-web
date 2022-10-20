use core::{convert::Infallible, future::Future};

use crate::{ready::ReadyService, service::Service};

/// A middleware unconditionally treat inner service type as ready.
/// See [ReadyService] for detail.
#[derive(Clone, Copy)]
pub struct UncheckedReady;

impl<S> Service<S> for UncheckedReady {
    type Response = UncheckedReadyService<S>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    fn call(&self, service: S) -> Self::Future<'_> {
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

impl<S> ReadyService for UncheckedReadyService<S> {
    type Ready = ();

    type ReadyFuture<'f> = impl Future<Output = Self::Ready>
    where
        S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async {}
    }
}
