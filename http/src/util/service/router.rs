use std::{collections::HashMap, error, fmt, future::Future};

use matchit::{MatchError, Node};
use xitca_service::{
    Request as ReqTrait, RequestSpecs, Service, ServiceFactory, ServiceFactoryExt, ServiceFactoryObject, ServiceObject,
    ServiceObjectTrait,
};

use crate::http::Request;

pub trait Routable {
    type Path<'a>: AsRef<str>
    where
        Self: 'a;
    fn path(&self) -> Self::Path<'_>;
}

/// Simple router for matching on [Request]'s path and call according service.
pub struct Router<Req, Res, Err, Cfg, InitErr> {
    routes: HashMap<&'static str, ServiceFactoryObject<Req, Res, Err, Cfg, InitErr>>,
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

impl<Req, Res, Err, Cfg, InitErr> Default for Router<Req, Res, Err, Cfg, InitErr> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Req, Res, Err, Cfg, InitErr> Router<Req, Res, Err, Cfg, InitErr> {
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
        F: ServiceFactory<Req, Response = Res, Error = Err, Config = Cfg, InitError = InitErr> + 'static,
        F::Future: 'static,
        // Bounds to satisfy ServiceObject
        Req: for<'a, 'b> ReqTrait<'a, &'a &'b ()>,
        F::Service: for<'a, 'b> ServiceObjectTrait<'a, &'a &'b (), Req, F::Response, F::Error>,
    {
        assert!(self.routes.insert(path, factory.into_object()).is_none());
        self
    }
}

impl<TrueReq, Req, Res, Err, Cfg, InitErr> ServiceFactory<TrueReq> for Router<Req, Res, Err, Cfg, InitErr>
where
    Cfg: Clone,
    ServiceObject<Req, Res, Err>: Service<TrueReq, Response = Res, Error = Err>,
    TrueReq: Routable,
{
    type Response = Res;
    type Error = RouterError<Err>;
    type Config = Cfg;
    type Service = RouterService<Req, Res, Err>;
    type InitError = InitErr;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let futs = self
            .routes
            .iter()
            .map(|(path, obj)| (*path, obj.new_service(cfg.clone())))
            .collect::<Vec<_>>();

        async move {
            let mut routes = matchit::Node::new();

            for (path, fut) in futs {
                let service = fut.await?;
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

impl<Req, TrueReq, Res, Err> Service<TrueReq> for RouterService<Req, Res, Err>
where
    //Req: RequestSpecs<TrueReq, Lifetime = &'a (), Lifetimes = Lt>,
    //Req: ReqTrait<'a, Lt, Type = TrueReq>,
    //ServiceObject<Req, Res, Err>: ServiceObjectTrait<'a, Lt, Req, Res, Err>,
    ServiceObject<Req, Res, Err>: Service<TrueReq, Response = Res, Error = Err>,
    TrueReq: Routable,
{
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
    fn call(&self, req: TrueReq) -> Self::Future<'_> {
        async move {
            let service = self
                .routes
                .at(req.path().as_ref())
                .map_err(RouterError::MatchError)?
                .value;

            Service::call(service, req).await.map_err(RouterError::Service)
        }
    }
}
