use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
};

use tokio::task::JoinHandle;
use xitca_io::net::{Listener, Stream};
use xitca_service::{ready::ReadyService, BuildService};

use crate::worker::{self, ServiceAny};

type LocalBoxFuture<'a, O> = Pin<Box<dyn Future<Output = O> + 'a>>;

pub(crate) struct Factory<F, Req> {
    inner: F,
    _t: PhantomData<Mutex<Req>>,
}

impl<F, Req> Factory<F, Req>
where
    F: BuildServiceSync<Req>,
    Req: From<Stream> + Send + 'static,
{
    pub(crate) fn new_boxed(inner: F) -> Box<dyn _BuildService> {
        Box::new(Self { inner, _t: PhantomData })
    }
}

type BuildServiceSyncOpt = Result<(Vec<JoinHandle<()>>, ServiceAny), ()>;

pub(crate) trait _BuildService: Send + Sync {
    fn _build<'s, 'f>(
        &'s self,
        name: &'f str,
        listeners: &'f [(String, Arc<Listener>)],
    ) -> LocalBoxFuture<'f, BuildServiceSyncOpt>
    where
        's: 'f;
}

impl<F, Req> _BuildService for Factory<F, Req>
where
    F: BuildServiceSync<Req>,
    F::Service: ReadyService<Req>,
    Req: From<Stream> + Send + 'static,
{
    fn _build<'s, 'f>(
        &'s self,
        name: &'f str,
        listeners: &'f [(String, Arc<Listener>)],
    ) -> LocalBoxFuture<'f, BuildServiceSyncOpt>
    where
        's: 'f,
    {
        Box::pin(async move {
            let service = self.inner.build().build(()).await.map_err(|_| ())?;
            let service = Rc::new(service);

            let handles = listeners
                .iter()
                .filter(|(n, _)| n == name)
                .map(|(_, listener)| worker::start(listener, &service))
                .collect::<Vec<_>>();

            Ok((handles, service as _))
        })
    }
}

/// Helper trait to cast a type that impl [`BuildService`](xitca_service::BuildService)
/// to a trait object that is `Send` and `Sync`.
pub trait BuildServiceSync<Req>
where
    Req: From<Stream>,
    Self: Send + Sync + 'static,
{
    type BuildService: BuildService<Service = Self::Service>;
    type Service: ReadyService<Req>;

    fn build(&self) -> Self::BuildService;
}

impl<F, T, Req> BuildServiceSync<Req> for F
where
    F: Fn() -> T + Send + Sync + 'static,
    T: BuildService,
    T::Service: ReadyService<Req>,
    Req: From<Stream>,
{
    type BuildService = T;
    type Service = T::Service;

    fn build(&self) -> T {
        self()
    }
}
