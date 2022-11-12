use std::{
    future::Future,
    marker::PhantomData,
    net::SocketAddr,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
};

use tokio::task::JoinHandle;
use xitca_io::net::{Listener, Stream};
use xitca_service::{ready::ReadyService, Service};

use crate::worker::{self, ServiceAny};

type LocalBoxFuture<'a, O> = Pin<Box<dyn Future<Output = O> + 'a>>;

pub(crate) struct Factory<F, Req> {
    inner: F,
    _t: PhantomData<Mutex<Req>>,
}

impl<F, Req> Factory<F, Req>
where
    F: BuildServiceFn<(Req, SocketAddr)>,
    Req: From<Stream> + Send + 'static,
{
    pub(crate) fn new_boxed(inner: F) -> Box<dyn _BuildService> {
        Box::new(Self { inner, _t: PhantomData })
    }
}

type BuildServiceSyncOpt = Result<(Vec<JoinHandle<()>>, ServiceAny), ()>;

// a specialized BuildService trait that can return a future that reference the input arguments.
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
    F: BuildServiceFn<(Req, SocketAddr)>,
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
            let service = self.inner.call().call(()).await.map_err(|_| ())?;
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

/// helper trait to alias impl Fn() -> impl BuildService type and hide it's generic type params(other than the Req type).
pub trait BuildServiceFn<Req>
where
    Self: Send + Sync + 'static,
{
    type BuildService: Service<Response = Self::Service>;
    type Service: ReadyService + Service<Req>;

    fn call(&self) -> Self::BuildService;
}

impl<F, T, Req> BuildServiceFn<Req> for F
where
    F: Fn() -> T + Send + Sync + 'static,
    T: Service,
    T::Response: ReadyService + Service<Req>,
{
    type BuildService = T;
    type Service = T::Response;

    fn call(&self) -> T {
        self()
    }
}
