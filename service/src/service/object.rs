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
pub struct ServiceObject<Req, Res, Err, G: ?Sized = DefaultObj<Req, Res, Err>>(
    Rc<G>,
    PhantomData<fn(Req) -> (Res, Err)>,
);

// TODO make ServiceObject generic over DefaultObj
type DefaultObj<ReqX, Res, Err> = dyn for<'a, 'b> ServiceObjectTrait<'a, &'a &'b (), ReqX, Res, Err>;

impl<Req, Res, Err, G: ?Sized> ServiceObject<Req, Res, Err, G> {
    pub(crate) fn new(obj: Rc<G>) -> Self {
        Self(obj, PhantomData)
    }
}

impl<Req, Res, Err> Clone for ServiceObject<Req, Res, Err> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1.clone())
    }
}

#[doc(hidden)]
pub trait ServiceObjectTrait<'a, Lt, ReqX: Request<'a, Lt>, Res, Err>: 'static {
    fn call(&self, req: ReqX::Type) -> BoxFuture<'a, Res, Err>;
}

impl<'a, Lt, S, ReqX, Res, Err> ServiceObjectTrait<'a, Lt, ReqX, Res, Err> for S
where
    ReqX: Request<'a, Lt>,
    S: Service<ReqX::Type, Response = Res, Error = Err> + Clone + 'static,
    S: Service<ReqX>, // inference; TODO remove
{
    #[inline]
    fn call(&self, req: ReqX::Type) -> BoxFuture<'a, Res, Err> {
        let this = self.clone();
        Box::pin(async move {
            //this.ready().await?;
            Service::call(&this, req).await
        })
    }
}

impl<'a, Lt, Req, TrueReq, Res, Err, G: ?Sized> Service<TrueReq> for ServiceObject<Req, Res, Err, G>
where
    Req: RequestSpecs<TrueReq, Lifetime = &'a (), Lifetimes = Lt>,
    Req: Request<'a, Lt, Type = TrueReq>,
    G: ServiceObjectTrait<'a, Lt, Req, Res, Err>,
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
    fn call(&self, req: TrueReq) -> Self::Future<'_> {
        async move { <G as ServiceObjectTrait<'a, Lt, Req, Res, Err>>::call(&self.0, req).await }
    }
}

#[test]
mod test_single_lifetime_request {
    use super::*;

    struct Req<'a>(&'a mut ());

    impl<'a, 'b> Request<'a, &'a &'b ()> for Req<'static> {
        type Type = Req<'a>;
    }

    impl<'a> RequestSpecs<Req<'a>> for Req<'static> {
        type Lifetime = &'a ();
        type Lifetimes = &'a &'static ();
    }

    fn check<T: for<'a> Service<Req<'a>>>() {}

    fn test() {
        check::<ServiceObject<Req<'static>, (), ()>>();
    }
}
