use core::future::Future;

use alloc::{boxed::Box, rc::Rc};

use crate::{service::ServiceObject, BoxFuture, Request, Service, ServiceObjectTrait};

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
    F::Future: 'static,
    // Bounds to satisfy ServiceObject
    Req: for<'a, 'b> Request<'a, &'a &'b ()>,
    F::Service: for<'a, 'b> ServiceObjectTrait<'a, &'a &'b (), Req, F::Response, F::Error>,
    //F::Service: Clone + 'static,
{
    type Future = BoxFuture<'static, ServiceObject<Req, F::Response, F::Error>, F::InitError>;

    fn new_service(&self, cfg: F::Config) -> Self::Future {
        let fut = ServiceFactory::new_service(self, cfg);
        Box::pin(async move {
            let service = fut.await?;
            Ok(ServiceObject::new(Rc::new(service) as _))
        })
    }
}
/*
impl<F, Req: Request, Svc, Fut, Res, Err, Cfg, InitErr> _ServiceFactoryObject<Req, Res, Err, Cfg, InitErr> for F
where
    Req: Request,
    F: for<'a> ServiceFactory<Req::Type<'a>, Service=Svc, Future=Fut, Config=Cfg, InitError=InitErr>,
    F: ServiceFactory<Req, Service=Svc>,
    Svc: Service<Req>, // Bound Serive
    F::Service: Clone + 'static,
    F::Future: 'static,
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
*/
