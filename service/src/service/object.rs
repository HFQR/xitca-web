use core::future::Future;

use alloc::{boxed::Box, rc::Rc};

use crate::BoxFuture;

use super::Service;

/// Trait object for type impls [Service] trait.
///
/// [Service] trait uses GAT which does not offer object safety.
/// This helper type offers the safety at the cost of tighter trait bound.
/// (Service type, input Request type and output future type must bound to 'static lifetime.)
pub type ServiceObject<Req, Res, Err> = Rc<dyn _ServiceObject<Req, Res, Err, Future = BoxFuture<'static, Res, Err>>>;

#[doc(hidden)]
pub trait _ServiceObject<Req, Res, Err> {
    type Future: Future<Output = Result<Res, Err>>;

    fn call(&self, req: Req) -> Self::Future;
}

impl<S, Req> _ServiceObject<Req, S::Response, S::Error> for S
where
    S: Service<Req> + Clone + 'static,
    Req: 'static,
{
    type Future = BoxFuture<'static, S::Response, S::Error>;

    #[inline]
    fn call(&self, req: Req) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            this.ready().await?;
            Service::call(&this, req).await
        })
    }
}
