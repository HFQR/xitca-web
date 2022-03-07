use std::{borrow::Borrow, collections::HashMap, error, fmt, future::Future, marker::PhantomData};

use matchit::{MatchError, Node};
use xitca_service::{
    ready::ReadyService, Service, ServiceFactory, ServiceFactoryExt, ServiceFactoryObject, ServiceObject,
};

use crate::http;

/// Simple router for matching on [Request]'s path and call according service.
pub struct Router<Req, ReqB, Arg, Res, Err> {
    routes: HashMap<&'static str, ServiceFactoryObject<Req, Arg, Res, Err>>,
    _req_body: PhantomData<ReqB>,
}

/// Error type of Router service.
pub enum RouterError<E> {
    /// Error occur on matching service.
    ///
    /// [MatchError::tsr] method can be used to hint if a route exists at the same path
    /// with/without a trailing slash.
    MatchError(MatchError),
    /// Error type of the inner service.
    Service(E),
}

impl<E: fmt::Debug> fmt::Debug for RouterError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MatchError(ref e) => write!(f, "{:?}", e),
            Self::Service(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl<E: fmt::Display> fmt::Display for RouterError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MatchError(ref e) => write!(f, "{}", e),
            Self::Service(ref e) => write!(f, "{}", e),
        }
    }
}

impl<E> error::Error for RouterError<E> where E: fmt::Debug + fmt::Display {}

impl<Req, ReqB, Arg, Res, Err> Default for Router<Req, ReqB, Arg, Res, Err> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Req, ReqB, Arg, Res, Err> Router<Req, ReqB, Arg, Res, Err> {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            _req_body: PhantomData,
        }
    }

    /// Insert a new service factory to given path.
    ///
    /// # Panic:
    ///
    /// When multiple services inserted with the same path.
    pub fn insert<F>(mut self, path: &'static str, factory: F) -> Self
    where
        F: ServiceFactory<Req, Arg, Response = Res, Error = Err> + 'static,
        F::Service: Clone + 'static,
        F::Future: 'static,
        Req: 'static,
    {
        assert!(self.routes.insert(path, factory.into_object()).is_none());
        self
    }
}

impl<Req, ReqB, Arg, Res, Err> ServiceFactory<Req, Arg> for Router<Req, ReqB, Arg, Res, Err>
where
    Req: Borrow<http::Request<ReqB>>,
    Arg: Clone,
{
    type Response = Res;
    type Error = RouterError<Err>;
    type Service = RouterService<Req, ReqB, Res, Err>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let futs = self
            .routes
            .iter()
            .map(|(path, obj)| (*path, obj.new_service(arg.clone())))
            .collect::<Vec<_>>();

        async move {
            let mut routes = matchit::Node::new();

            for (path, fut) in futs {
                let service = fut.await.map_err(RouterError::Service)?;
                routes.insert(path, service).unwrap();
            }

            Ok(RouterService {
                routes,
                _req_body: PhantomData,
            })
        }
    }
}

pub struct RouterService<Req, ReqB, Res, Err> {
    routes: Node<ServiceObject<Req, Res, Err>>,
    _req_body: PhantomData<ReqB>,
}

impl<Req, ReqB, Res, Err> Clone for RouterService<Req, ReqB, Res, Err> {
    fn clone(&self) -> Self {
        Self {
            routes: self.routes.clone(),
            _req_body: PhantomData,
        }
    }
}

impl<Req, ReqB, Res, Err> Service<Req> for RouterService<Req, ReqB, Res, Err>
where
    Req: Borrow<http::Request<ReqB>>,
{
    type Response = Res;
    type Error = RouterError<Err>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let service = self
                .routes
                .at(req.borrow().uri().path())
                .map_err(RouterError::MatchError)?;

            service.value.call(req).await.map_err(RouterError::Service)
        }
    }
}

impl<Req, ReqB, Res, Err> ReadyService<Req> for RouterService<Req, ReqB, Res, Err>
where
    Req: Borrow<http::Request<ReqB>>,
{
    type Ready = ();
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async { Ok(()) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::convert::Infallible;

    use xitca_service::fn_service;

    use crate::{
        http::{self, Response},
        request::Request,
    };

    #[tokio::test]
    async fn router_accept_crate_request() {
        Router::new()
            .insert(
                "/",
                fn_service(|_: Request<()>| async { Ok::<_, Infallible>(Response::new(())) }),
            )
            .new_service(())
            .await
            .unwrap()
            .call(Request::new(()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn router_accept_http_request() {
        Router::new()
            .insert(
                "/",
                fn_service(|_: http::Request<()>| async { Ok::<_, Infallible>(Response::new(())) }),
            )
            .new_service(())
            .await
            .unwrap()
            .call(http::Request::new(()))
            .await
            .unwrap();
    }
}
