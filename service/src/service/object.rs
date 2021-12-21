use alloc::{boxed::Box, rc::Rc};
use core::future::Future;
use core::marker::PhantomData;

use crate::{BoxFuture, Request, RequestSpecs};

use super::Service;

/// Trait object for type impls [Service] trait.
///
/// [Service] trait uses GAT which does not offer object safety.
/// This helper type offers the safety at the cost of tighter trait bound.
/// (Service type, input Request type and output future type must bound to 'static lifetime.)
pub struct ServiceObject<ReqS, Res, Err, G: ?Sized>(Rc<G>, PhantomData<fn(ReqS) -> (Res, Err)>);

impl<ReqS, Res, Err, G: ?Sized> ServiceObject<ReqS, Res, Err, G> {
    pub(crate) fn new(obj: Rc<G>) -> Self {
        Self(obj, PhantomData)
    }
}

impl<ReqS, Res, Err, G: ?Sized> Clone for ServiceObject<ReqS, Res, Err, G> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1.clone())
    }
}

#[doc(hidden)]
pub trait ServiceObjectTrait<'a, Lt, ReqS: Request<'a, Lt>, Res, Err>: 'static {
    fn call(&self, req: ReqS::Type) -> BoxFuture<'a, Res, Err>;
}

impl<'a, Lt, S, ReqS, Res, Err> ServiceObjectTrait<'a, Lt, ReqS, Res, Err> for S
where
    ReqS: Request<'a, Lt>,
    S: Service<ReqS::Type, Response = Res, Error = Err> + Clone + 'static,
    // S: Service<ReqS>, // Consider in case of failed inference for ReqS
{
    #[inline]
    fn call(&self, req: ReqS::Type) -> BoxFuture<'a, Res, Err> {
        let this = self.clone();
        Box::pin(async move {
            this.ready().await?;
            Service::call(&this, req).await
        })
    }
}

impl<'a, Lt, ReqS, Req, Res, Err, G> Service<Req> for ServiceObject<ReqS, Res, Err, G>
where
    ReqS: RequestSpecs<Req, Lifetime = &'a (), Lifetimes = Lt>,
    ReqS: Request<'a, Lt, Type = Req>,
    G: ?Sized + ServiceObjectTrait<'a, Lt, ReqS, Res, Err>,
{
    type Response = Res;
    type Error = Err;
    type Ready<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline(always)]
    fn ready(&self) -> Self::Ready<'_> {
        async { Ok(()) }
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move { ServiceObjectTrait::call(&*self.0, req).await }
    }
}
