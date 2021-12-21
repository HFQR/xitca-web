use std::{collections::HashMap, error, fmt, future::Future};

use matchit::{MatchError, Node};
use xitca_service::{Service, ServiceFactory, ServiceFactoryExt, ServiceFactoryObject, ServiceFactoryObjectTrait};

pub trait Routable {
    type Path<'a>: AsRef<str>
    where
        Self: 'a;

    fn path(&self) -> Self::Path<'_>;
}

/// Simple router for matching on [Request]'s path and call according service.
pub struct Router<F> {
    routes: HashMap<&'static str, F>,
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

impl<Req, Res, Err, Cfg, InitErr, SO> Default for Router<ServiceFactoryObject<Req, Res, Err, Cfg, InitErr, SO>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Req, Res, Err, Cfg, InitErr, SO> Router<ServiceFactoryObject<Req, Res, Err, Cfg, InitErr, SO>> {
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
        F: ServiceFactoryObjectTrait<Req, Res, Err, Cfg, InitErr, ServiceObj = SO>,
    {
        assert!(self.routes.insert(path, factory.into_object()).is_none());
        self
    }
}

impl<Req, F, Cfg> ServiceFactory<Req> for Router<F>
where
    Req: Routable,
    F: ServiceFactory<Req, Config = Cfg>,
    Cfg: Clone,
{
    type Response = F::Response;
    type Error = RouterError<F::Error>;
    type Config = F::Config;
    type Service = RouterService<F::Service>;
    type InitError = F::InitError;
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

pub struct RouterService<S> {
    routes: Node<S>,
}

impl<S: Clone> Clone for RouterService<S> {
    fn clone(&self) -> Self {
        Self {
            routes: self.routes.clone(),
        }
    }
}

impl<Req, S> Service<Req> for RouterService<S>
where
    Req: Routable,
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = RouterError<S::Error>;
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
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let service = self
                .routes
                .at(req.path().as_ref())
                .map_err(RouterError::MatchError)?
                .value;

            service.call(req).await.map_err(RouterError::Service)
        }
    }
}
