use alloc::boxed::Box;

use crate::BoxFuture;

use super::Service;

/// Trait object for type impls [Service] trait.
///
/// [Service] trait uses GAT which does not offer object safety.
pub type ServiceObject<Req, Res, Err> = Box<dyn _ServiceObject<Req, Res, Err>>;

#[doc(hidden)]
pub trait _ServiceObject<Req, Res, Err> {
    fn call<'s, 'f>(&'s self, req: Req) -> BoxFuture<'f, Res, Err>
    where
        Req: 'f,
        's: 'f;
}

impl<S, Req> _ServiceObject<Req, S::Response, S::Error> for S
where
    S: Service<Req>,
{
    #[inline]
    fn call<'s, 'f>(&'s self, req: Req) -> BoxFuture<'f, S::Response, S::Error>
    where
        Req: 'f,
        's: 'f,
    {
        Box::pin(async move { Service::call(self, req).await })
    }
}
