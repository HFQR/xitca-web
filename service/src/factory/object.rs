use alloc::{boxed::Box, rc::Rc};

use crate::{service::ServiceObject, BoxFuture};

use super::ServiceFactory;

/// Trait object for constructing [ServiceObject].
pub type ServiceFactoryObject<Req, Arg, Res, Err> = Box<dyn _ServiceFactoryObject<Req, Arg, Res, Err>>;

#[doc(hidden)]
pub trait _ServiceFactoryObject<Req, Arg, Res, Err> {
    fn new_service(&self, arg: Arg) -> BoxFuture<'static, ServiceObject<Req, Res, Err>, Err>;
}

impl<F, Req, Arg> _ServiceFactoryObject<Req, Arg, F::Response, F::Error> for F
where
    F: ServiceFactory<Req, Arg>,
    F::Service: Clone + 'static,
    F::Future: 'static,
    Req: 'static,
{
    fn new_service(&self, arg: Arg) -> BoxFuture<'static, ServiceObject<Req, F::Response, F::Error>, F::Error> {
        let fut = ServiceFactory::new_service(self, arg);
        Box::pin(async move {
            let service = fut.await?;
            Ok(Rc::new(service) as _)
        })
    }
}
