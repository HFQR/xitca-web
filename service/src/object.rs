use alloc::boxed::Box;

use super::{service::Service, BoxFuture};

/// Object-safe counterpart of [Service].
pub trait ServiceObject<Req> {
    type Response;
    type Error;

    fn call<'s>(&'s self, req: Req) -> BoxFuture<'s, Self::Response, Self::Error>
    where
        Req: 's;
}

impl<S, Req> ServiceObject<Req> for S
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> BoxFuture<'s, Self::Response, Self::Error>
    where
        Req: 's,
    {
        Box::pin(Service::call(self, req))
    }
}

impl<I, Req> Service<Req> for Box<I>
where
    I: ServiceObject<Req> + ?Sized,
{
    type Response = I::Response;
    type Error = I::Error;
    type Future<'f> = BoxFuture<'f, Self::Response, Self::Error> where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        ServiceObject::call(&**self, req)
    }
}

/// An often used type alias for boxed service object. used when Req type is not bound to any
/// lifetime.
pub type BoxedServiceObject<Req, Res, Err> = Box<dyn ServiceObject<Req, Response = Res, Error = Err>>;
