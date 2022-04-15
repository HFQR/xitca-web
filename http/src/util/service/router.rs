use std::{borrow::BorrowMut, collections::HashMap, error, fmt, future::Future, marker::PhantomData};

use matchit::{MatchError, Node};
use xitca_service::{
    object::{DefaultFactoryObject, DefaultObjectConstructor, ObjectConstructor},
    ready::ReadyService,
    Service, ServiceFactory,
};

use crate::http;

/// A [GenericRouter] specialized with [DefaultObjectConstructor]
pub type Router<Req, Arg, Res, Err, ReqB> =
    GenericRouter<DefaultObjectConstructor<Req, Arg>, DefaultFactoryObject<Req, Arg, Res, Err>, ReqB>;

/// Simple router for matching on [Request]'s path and call according service.
///
/// An [ObjectConstructor] must be specified as a type prameter
/// in order to determine how the router type-erases node services.
pub struct GenericRouter<ObjCons, SF, ReqB> {
    routes: HashMap<&'static str, SF>,
    _req_body: PhantomData<(ObjCons, ReqB)>,
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

impl<ObjCons, SF, ReqB> Default for GenericRouter<ObjCons, SF, ReqB> {
    fn default() -> Self {
        Self::new()
    }
}

impl<ObjCons, SF, ReqB> GenericRouter<ObjCons, SF, ReqB> {
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
        ObjCons: ObjectConstructor<F, Object = SF>,
    {
        assert!(self.routes.insert(path, ObjCons::into_object(factory)).is_none());
        self
    }
}

impl<ObjCons, SF, Req, ReqB, Arg> ServiceFactory<Req, Arg> for GenericRouter<ObjCons, SF, ReqB>
where
    SF: ServiceFactory<Req, Arg>,
    Req: BorrowMut<http::Request<ReqB>>,
    Arg: Clone,
{
    type Response = SF::Response;
    type Error = RouterError<SF::Error>;
    type Service = RouterService<SF::Service, ReqB>;
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

pub struct RouterService<S, ReqB> {
    routes: Node<S>,
    _req_body: PhantomData<ReqB>,
}

impl<S, Req, ReqB> Service<Req> for RouterService<S, ReqB>
where
    S: Service<Req>,
    Req: BorrowMut<http::Request<ReqB>>,
{
    type Response = S::Response;
    type Error = RouterError<S::Error>;
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

impl<S, Req, ReqB> ReadyService<Req> for RouterService<S, ReqB>
where
    S: ReadyService<Req>,
    Req: BorrowMut<http::Request<ReqB>>,
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

    use xitca_service::{fn_service, Service, ServiceFactoryExt};

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

    #[tokio::test]
    async fn router_enclosed_fn() {
        async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
        where
            S: Service<Req>,
        {
            service.call(req).await
        }

        Router::new()
            .insert(
                "/",
                fn_service(|_: http::Request<()>| async { Ok::<_, Infallible>(Response::new(())) }),
            )
            .enclosed_fn(enclosed)
            .new_service(())
            .await
            .unwrap()
            .call(http::Request::new(()))
            .await
            .unwrap();
    }
}
