use core::{future::Future, marker::PhantomData, pin::Pin};

use std::{rc::Rc, sync::Arc};

use tokio::task::JoinHandle;
use xitca_io::net::{Listener, Stream};
use xitca_service::{
    object::{BoxedServiceObject, ServiceObject},
    ready::ReadyService,
    Service,
};

use crate::worker::{self, ServiceAny};

struct Builder<F, Req> {
    inner: F,
    _t: PhantomData<fn(Req)>,
}

type Res = (Vec<JoinHandle<()>>, ServiceAny);
type Arg<'a> = (&'a str, &'a [(String, Arc<Listener>)]);

pub(crate) type BuildServiceObj =
    BoxedServiceObject<dyn for<'r> ServiceObject<Arg<'r>, Response = Res, Error = ()> + Send + Sync>;

impl<'r, F, Req> ServiceObject<Arg<'r>> for Builder<F, Req>
where
    F: BuildServiceFn<Req>,
    Req: From<Stream> + 'static,
{
    type Response = Res;
    type Error = ();

    fn call<'s>(
        &'s self,
        (name, listeners): Arg<'r>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + 's>>
    where
        'r: 's,
    {
        Box::pin(async move {
            let build = self.inner.call();
            let service = Service::call(&build, ()).await.map_err(|_| ())?;
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
        BoxedServiceObject(Box::new(Builder {
            inner: self,
            _t: PhantomData,
        }))
    }
}
