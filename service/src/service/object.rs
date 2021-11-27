use core::{
    future::Future,
    ops::{Deref, DerefMut},
};

use alloc::boxed::Box;

use crate::BoxFuture;

use super::Service;

/// Trait object for type impls [Service] trait.
///
/// [Service] trait uses GAT which does not offer object safety.
/// This helper type offers the safety at the cost of tighter trait bound.
/// (Service type, input Request type and output future type must bound to 'static lifetime.)
pub struct ServiceObject<Req, Res, Err>(Box<dyn _ServiceObject<Req, Res, Err, Future = BoxFuture<'static, Res, Err>>>);

impl<Req, Res, Err> Deref for ServiceObject<Req, Res, Err> {
    type Target = dyn _ServiceObject<Req, Res, Err, Future = BoxFuture<'static, Res, Err>>;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl<Req, Res, Err> DerefMut for ServiceObject<Req, Res, Err> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl<Req, Res, Err> ServiceObject<Req, Res, Err> {
    pub(crate) fn new<S>(service: S) -> Self
    where
        S: Service<Req, Response = Res, Error = Err> + Clone + 'static,
        Req: 'static,
    {
        Self(Box::new(service))
    }
}

impl<Req, Res, Err> Clone for ServiceObject<Req, Res, Err> {
    fn clone(&self) -> Self {
        self.0.clone_object()
    }
}

#[doc(hidden)]
pub trait _ServiceObject<Req, Res, Err> {
    type Future: Future<Output = Result<Res, Err>>;

    fn clone_object(&self) -> ServiceObject<Req, Res, Err>;

    fn call(&self, req: Req) -> Self::Future;
}

impl<S, Req> _ServiceObject<Req, S::Response, S::Error> for S
where
    S: Service<Req> + Clone + 'static,
    Req: 'static,
{
    type Future = BoxFuture<'static, S::Response, S::Error>;

    #[inline]
    fn clone_object(&self) -> ServiceObject<Req, S::Response, S::Error> {
        ServiceObject(Box::new(self.clone()) as _)
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            this.ready().await?;
            Service::call(&this, req).await
        })
    }
}
