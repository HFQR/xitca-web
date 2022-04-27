use std::{marker::PhantomData, rc::Rc, sync::Arc};

use futures_core::future::LocalBoxFuture;
use tokio::task::JoinHandle;
use xitca_io::net::{Listener, Stream};
use xitca_service::{ready::ReadyService, BuildService};

use crate::worker::{self, Counter};

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

    fn spawn_handle<'s, 'f>(
        &'s self,
        name: &'f str,
        listeners: &'f [(String, Arc<Listener>)],
        counter: &'f Counter,
    ) -> LocalBoxFuture<'f, Result<Vec<JoinHandle<()>>, ()>>
    where
        's: 'f;
}

impl<F, Req> ServiceFactoryClone for Factory<F, Req>
where
    F: AsServiceFactoryClone<Req>,
    F::Service: ReadyService<Req>,
    Req: From<Stream> + Send + 'static,
{
    fn clone_factory(&self) -> Box<dyn ServiceFactoryClone> {
        Box::new(Self {
            inner: self.inner.clone(),
            _t: PhantomData,
        })
    }

    fn spawn_handle<'s, 'f>(
        &'s self,
        name: &'f str,
        listeners: &'f [(String, Arc<Listener>)],
        counter: &'f Counter,
    ) -> LocalBoxFuture<'f, Result<Vec<JoinHandle<()>>, ()>>
    where
        's: 'f,
    {
        Box::pin(async move {
            let service = self.inner.as_factory_clone().build(()).await.map_err(|_| ())?;
            let service = Rc::new(service);

            let handles = listeners
                .iter()
                .filter(|(n, _)| n == name)
                .map(|(_, listener)| worker::start(listener, &service, counter))
                .collect::<Vec<_>>();

            Ok(handles)
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
    type ServiceFactoryClone: BuildService<Service = Self::Service>;
    type Service: ReadyService<Req>;

    fn as_factory_clone(&self) -> Self::ServiceFactoryClone;
}

impl<F, T, Req> AsServiceFactoryClone<Req> for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: BuildService,
    T::Service: ReadyService<Req>,
    Req: From<Stream>,
{
    type ServiceFactoryClone = T;
    type Service = T::Service;

    fn as_factory_clone(&self) -> T {
        (self)()
    }
}
