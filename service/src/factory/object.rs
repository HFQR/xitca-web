use alloc::{boxed::Box, rc::Rc};

use crate::{service::ServiceObject, BoxFuture, Request, Service, ServiceObjectTrait};

use super::ServiceFactory;

/// Trait object for constructing [ServiceObject].
pub struct ServiceFactoryObject<ReqS, Res, Err, Cfg, InitErr, SO>(
    pub(crate) Box<dyn ServiceFactoryObjectTrait<ReqS, Res, Err, Cfg, InitErr, ServiceObj = SO>>,
);

#[doc(hidden)]
pub trait ServiceFactoryObjectTrait<Req, Res, Err, Cfg, InitErr> {
    type ServiceObj;

    fn obj_new_service(&self, cfg: Cfg) -> BoxFuture<'static, Self::ServiceObj, InitErr>;
}

impl<F, ReqS> ServiceFactoryObjectTrait<ReqS, F::Response, F::Error, F::Config, F::InitError> for F
where
    F: ServiceFactory<ReqS>,
    F::Future: 'static,
    // Bounds to satisfy ServiceObject
    ReqS: for<'a, 'b> Request<'a, &'a &'b ()>,
    F::Service: for<'a, 'b> ServiceObjectTrait<'a, &'a &'b (), ReqS, F::Response, F::Error>,
{
    type ServiceObj = ServiceObject<
        ReqS,
        F::Response,
        F::Error,
        dyn for<'a, 'b> ServiceObjectTrait<'a, &'a &'b (), ReqS, F::Response, F::Error>,
    >;

    fn obj_new_service(&self, cfg: F::Config) -> BoxFuture<'static, Self::ServiceObj, F::InitError> {
        let fut = self.new_service(cfg);
        Box::pin(async move {
            let service = fut.await?;
            Ok(ServiceObject::new(Rc::new(service) as _))
        })
    }
}

impl<ReqS, Req, Res, Err, Cfg, InitErr, SO> ServiceFactory<Req>
    for ServiceFactoryObject<ReqS, Res, Err, Cfg, InitErr, SO>
where
    SO: Service<Req, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = Err;
    type Config = Cfg;
    type Service = SO;
    type InitError = InitErr;
    type Future = BoxFuture<'static, Self::Service, Self::InitError>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        self.0.obj_new_service(cfg)
    }
}
