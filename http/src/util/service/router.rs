use std::{collections::HashMap, error, fmt, future::Future};

use matchit::{MatchError, Node};
use xitca_service::{Service, ServiceFactory, ServiceFactoryExt, ServiceFactoryObject, ServiceObject};

use crate::request::Request;

/// Simple router for matching on [Request]'s path and call according service.
pub struct Router<Req, Arg, Res, Err> {
    routes: HashMap<&'static str, ServiceFactoryObject<Req, Arg, Res, Err>>,
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

impl<Req, Arg, Res, Err> Default for Router<Req, Arg, Res, Err> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Req, Arg, Res, Err> Router<Req, Arg, Res, Err> {
    pub fn new() -> Self {
        Self { routes: HashMap::new() }
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

impl<ReqB, Arg, Res, Err> ServiceFactory<Request<ReqB>, Arg> for Router<Request<ReqB>, Arg, Res, Err>
where
    Arg: Clone,
{
    type Response = Res;
    type Error = RouterError<Err>;
    type Service = RouterService<Request<ReqB>, Res, Err>;
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

            Ok(RouterService { routes })
        }
    }
}

pub struct RouterService<Req, Res, Err> {
    routes: Node<ServiceObject<Req, Res, Err>>,
}

impl<Req, Res, Err> Clone for RouterService<Req, Res, Err> {
    fn clone(&self) -> Self {
        Self {
            routes: self.routes.clone(),
        }
    }
}

impl<ReqB, Res, Err> Service<Request<ReqB>> for RouterService<Request<ReqB>, Res, Err> {
    type Response = Res;
    type Error = RouterError<Err>;
    type Ready<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline(always)]
    fn ready(&self) -> Self::Ready<'_> {
        async { Ok(()) }
    }

    #[inline]
    fn call(&self, req: Request<ReqB>) -> Self::Future<'_> {
        async move {
            let service = self.routes.at(req.uri().path()).map_err(RouterError::MatchError)?;

            service.value.call(req).await.map_err(RouterError::Service)
        }
    }
}
