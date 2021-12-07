use std::marker::PhantomData;

use futures_core::future::LocalBoxFuture;
use xitca_io::net::Stream;
use xitca_service::ServiceFactory;

use crate::worker::{RcWorkerService, WorkerService};

pub(crate) struct Factory<F, Req> {
    inner: F,
    _t: PhantomData<Req>,
}

impl<F, Req> Factory<F, Req>
where
    F: AsServiceFactoryClone<Req>,
    Req: From<Stream> + Send + 'static,
{
    pub(crate) fn new_boxed(inner: F) -> Box<dyn ServiceFactoryClone> {
        Box::new(Self { inner, _t: PhantomData })
    }
}

pub(crate) trait ServiceFactoryClone: Send {
    fn clone_factory(&self) -> Box<dyn ServiceFactoryClone>;

    fn new_service(&self) -> LocalBoxFuture<'static, Result<RcWorkerService, ()>>;
}

impl<F, Req> ServiceFactoryClone for Factory<F, Req>
where
    F: AsServiceFactoryClone<Req>,
    Req: From<Stream> + Send + 'static,
{
    fn clone_factory(&self) -> Box<dyn ServiceFactoryClone> {
        Box::new(Self {
            inner: self.inner.clone(),
            _t: PhantomData,
        })
    }

    fn new_service(&self) -> LocalBoxFuture<'static, Result<RcWorkerService, ()>> {
        let fut = self.inner.as_factory_clone().new_service(());
        Box::pin(async move {
            let service = fut.await.map_err(|_| ())?;

            Ok(WorkerService::new_rcboxed(service))
        })
    }
}

/// Helper trait to cast a cloneable type that impl [`ServiceFactory`](xitca_service::ServiceFactory)
/// to a trait object that is `Send` and `Clone`.
pub trait AsServiceFactoryClone<Req>
where
    Req: From<Stream>,
    Self: Send + Clone + 'static,
{
    type ServiceFactoryClone: ServiceFactory<Req, Config = ()>;

    fn as_factory_clone(&self) -> Self::ServiceFactoryClone;
}

impl<F, T, Req> AsServiceFactoryClone<Req> for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: ServiceFactory<Req, Config = ()>,
    Req: From<Stream>,
{
    type ServiceFactoryClone = T;

    fn as_factory_clone(&self) -> T {
        (self)()
    }
}
