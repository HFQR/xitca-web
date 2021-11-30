use core::future::Future;

use alloc::{boxed::Box, rc::Rc};

use crate::{service::ServiceObject, BoxFuture};

use super::ServiceFactory;

/// Trait object for constructing [ServiceObject].
pub type ServiceFactoryObject<Req, Res, Err, Cfg, InitErr> = Box<
    dyn _ServiceFactoryObject<
        Req,
        Res,
        Err,
        Cfg,
        InitErr,
        Future = BoxFuture<'static, ServiceObject<Req, Res, Err>, InitErr>,
    >,
>;

#[doc(hidden)]
pub trait _ServiceFactoryObject<Req, Res, Err, Cfg, InitErr> {
    type Future: Future<Output = Result<ServiceObject<Req, Res, Err>, InitErr>>;

    fn new_service(&self, cfg: Cfg) -> Self::Future;
}

impl<F, Req> _ServiceFactoryObject<Req, F::Response, F::Error, F::Config, F::InitError> for F
where
    F: ServiceFactory<Req>,
    F::Service: Clone + 'static,
    F::Future: 'static,
    Req: 'static,
{
    type Future = BoxFuture<'static, ServiceObject<Req, F::Response, F::Error>, F::InitError>;

    fn new_service(&self, cfg: F::Config) -> Self::Future {
        let fut = ServiceFactory::new_service(self, cfg);
        Box::pin(async move {
            let service = fut.await?;
            Ok(Rc::new(service) as _)
        })
    }
}
