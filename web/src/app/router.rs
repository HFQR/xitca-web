use std::borrow::Cow;

use xitca_http::util::service::router::{IntoObject, Router, RouterError, RouterGen, RouterMapErr, TypedRoute};

use crate::{
    error::Error,
    service::{ready::ReadyService, Service},
    WebContext,
};

/// application wrap around [Router] and transform it's error type into [Error]
pub struct AppRouter<Obj>(Router<Obj>);

impl<Obj> AppRouter<Obj> {
    pub(super) fn new() -> Self {
        Self(Router::new())
    }

    pub(super) fn insert<F, Arg, Req>(mut self, path: &'static str, builder: F) -> Self
    where
        F: Service<Arg> + RouterGen + Send + Sync,
        F::Response: Service<Req>,
        Req: IntoObject<F::Route<F>, Arg, Object = Obj>,
    {
        self.0 = self.0.insert(path, builder);
        self
    }

    pub(super) fn insert_typed<T, M>(mut self, t: T) -> Self
    where
        T: TypedRoute<M, Route = Obj>,
    {
        self.0 = self.0.insert_typed(t);
        self
    }
}

impl<Obj> RouterGen for AppRouter<Obj> {
    type Route<R1> = RouterMapErr<<Router<Obj> as RouterGen>::Route<R1>>;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.0.path_gen(prefix)
    }

    fn route_gen<R1>(route: R1) -> Self::Route<R1> {
        RouterMapErr(<Router<Obj> as RouterGen>::route_gen(route))
    }
}

impl<Arg, Obj> Service<Arg> for AppRouter<Obj>
where
    Router<Obj>: Service<Arg>,
{
    type Response = RouterService<<Router<Obj> as Service<Arg>>::Response>;
    type Error = <Router<Obj> as Service<Arg>>::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        self.0.call(arg).await.map(RouterService)
    }
}

pub struct RouterService<S>(S);

impl<'r, S, C, B, Res, E> Service<WebContext<'r, C, B>> for RouterService<S>
where
    S: for<'r2> Service<WebContext<'r2, C, B>, Response = Res, Error = RouterError<E>>,
    Error<C>: From<E>,
{
    type Response = Res;
    type Error = Error<C>;

    #[inline]
    async fn call(&self, req: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.0.call(req).await.map_err(|e| match e {
            RouterError::Match(e) => Error::from_service(e),
            RouterError::NotAllowed(e) => Error::from_service(e),
            RouterError::Service(e) => Error::from(e),
        })
    }
}

impl<S> ReadyService for RouterService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.0.ready().await
    }
}
