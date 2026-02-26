use core::marker::PhantomData;

use std::rc::Rc;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use xitca_io::net::Stream;
use xitca_service::{Service, ready::ReadyService};

use crate::{
    net::ListenerDyn,
    worker::{self, ServiceAny},
};

pub type ServiceObj = Box<
    dyn for<'a> xitca_service::object::ServiceObject<
            (&'a str, &'a [(String, ListenerDyn)], CancellationToken),
            Response = (Vec<JoinHandle<()>>, ServiceAny),
            Error = (),
        > + Send
        + Sync,
>;

struct Container<F, Req> {
    inner: F,
    _t: PhantomData<fn(Req)>,
}

impl<'a, F, Req> Service<(&'a str, &'a [(String, ListenerDyn)], CancellationToken)> for Container<F, Req>
where
    F: IntoServiceObj<Req>,
    Req: TryFrom<Stream> + 'static,
{
    type Response = (Vec<JoinHandle<()>>, ServiceAny);
    type Error = ();

    async fn call(
        &self,
        (name, listeners, cancellation_token): (&'a str, &'a [(String, ListenerDyn)], CancellationToken),
    ) -> Result<Self::Response, Self::Error> {
        let service = self.inner.call(()).await.map_err(|_| ())?;
        let service = Rc::new(service);

        let handles = listeners
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, listener)| worker::start(listener, &service, cancellation_token.clone()))
            .collect::<Vec<_>>();

        Ok((handles, service as _))
    }
}

/// helper trait for erase generic params of [Service]
pub trait IntoServiceObj<Req>: Send + Sync + 'static
where
    Self: Service<Response = Self::Service> + Send + Sync + 'static,
    Req: TryFrom<Stream> + 'static,
{
    type Service: ReadyService + Service<(Req, CancellationToken)>;

    fn into_object(self) -> ServiceObj;
}

impl<T, Req> IntoServiceObj<Req> for T
where
    T: Service + Send + Sync + 'static,
    T::Response: ReadyService + Service<(Req, CancellationToken)>,
    Req: TryFrom<Stream> + 'static,
{
    type Service = T::Response;

    fn into_object(self) -> ServiceObj {
        Box::new(Container {
            inner: self,
            _t: PhantomData,
        })
    }
}
