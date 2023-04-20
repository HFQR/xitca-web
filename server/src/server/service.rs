use std::{future::Future, marker::PhantomData, pin::Pin, rc::Rc, sync::Arc};

use tokio::task::JoinHandle;
use xitca_io::net::{Listener, Stream};
use xitca_service::{ready::ReadyService, Service};

use crate::worker::{self, ServiceAny};

type LocalBoxFuture<'a, O> = Pin<Box<dyn Future<Output = O> + 'a>>;

type BuildServiceSyncOpt = Result<(Vec<JoinHandle<()>>, ServiceAny), ()>;

pub type BuildServiceObj = Box<dyn BuildService + Send + Sync>;

// a specialized BuildService trait that can return a future that reference the input arguments.
pub trait BuildService {
    fn call<'s, 'f>(&'s self, arg: (&'f str, &'f [(String, Arc<Listener>)])) -> LocalBoxFuture<'f, BuildServiceSyncOpt>
    where
        's: 'f;
}

struct Container<F, Req> {
    inner: F,
    _t: PhantomData<fn(Req)>,
}

impl<F, Req> BuildService for Container<F, Req>
where
    F: BuildServiceFn<Req>,
    Req: From<Stream> + 'static,
{
    fn call<'s, 'f>(
        &'s self,
        (name, listeners): (&'f str, &'f [(String, Arc<Listener>)]),
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
pub trait BuildServiceFn<Req>: Send + Sync + 'static {
    type BuildService: Service<Response = Self::Service>;
    type Service: ReadyService + Service<Req>;

    fn call(&self) -> Self::BuildService;

    fn into_object(self) -> BuildServiceObj;
}

impl<F, T, Req> BuildServiceFn<Req> for F
where
    F: Fn() -> T + Send + Sync + 'static,
    T: Service,
    T::Response: ReadyService + Service<Req>,
    Req: From<Stream> + 'static,
{
    type BuildService = T;
    type Service = T::Response;

    fn call(&self) -> T {
        self()
    }

    fn into_object(self) -> BuildServiceObj {
        Box::new(Container {
            inner: self,
            _t: PhantomData,
        })
    }
}
