use std::marker::PhantomData;

use actix_service::ServiceFactory;
use futures_core::future::LocalBoxFuture;

use crate::net::FromStream;
use crate::worker::{RcWorkerService, WorkerService};

pub(crate) struct Factory<F, Req>
where
    F: IntoServiceFactoryClone<Req>,
    Req: FromStream + Send,
{
    inner: F,
    _t: PhantomData<Req>,
}

impl<F, Req> Factory<F, Req>
where
    F: IntoServiceFactoryClone<Req>,
    Req: FromStream + Send + 'static,
{
    pub(crate) fn create(inner: F) -> Box<dyn ServiceFactoryClone> {
        Box::new(Self {
            inner,
            _t: PhantomData,
        })
    }
}

pub(crate) trait ServiceFactoryClone: Send {
    fn clone_factory(&self) -> Box<dyn ServiceFactoryClone>;

    fn create(&self) -> LocalBoxFuture<'static, Result<RcWorkerService, ()>>;
}

impl<F, Req> ServiceFactoryClone for Factory<F, Req>
where
    F: IntoServiceFactoryClone<Req>,
    Req: FromStream + Send + 'static,
{
    fn clone_factory(&self) -> Box<dyn ServiceFactoryClone> {
        Box::new(Self {
            inner: self.inner.clone(),
            _t: PhantomData,
        })
    }

    fn create(&self) -> LocalBoxFuture<'static, Result<RcWorkerService, ()>> {
        let fut = self.inner.create().new_service(());
        Box::pin(async move {
            let service = fut.await.map_err(|_| ())?;

            Ok(WorkerService::new_rcboxed(service))
        })
    }
}

pub trait IntoServiceFactoryClone<Req>
where
    Req: FromStream,
    Self: Send + Clone + 'static,
{
    type Factory: ServiceFactory<Req, Config = ()>;

    fn create(&self) -> Self::Factory;
}

impl<F, T, Req> IntoServiceFactoryClone<Req> for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: ServiceFactory<Req, Config = ()>,
    Req: FromStream,
{
    type Factory = T;

    fn create(&self) -> T {
        (self)()
    }
}
