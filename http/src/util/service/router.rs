use std::{collections::HashMap, future::Future, marker::PhantomData};

use matchit::{MatchError, Node};
use xitca_service::{
    object::{DefaultFactoryObject, DefaultObjectConstructor, ObjectConstructor},
    pipeline::PipelineE,
    ready::ReadyService,
    Service, ServiceFactory,
};

use crate::{http, request::BorrowReq};

/// A [GenericRouter] specialized with [DefaultObjectConstructor]
pub type Router<Req, Arg, Res, Err> =
    GenericRouter<DefaultObjectConstructor<Req, Arg>, DefaultFactoryObject<Req, Arg, Res, Err>>;

/// Simple router for matching on [Request]'s path and call according service.
///
/// An [ObjectConstructor] must be specified as a type prameter
/// in order to determine how the router type-erases node services.
pub struct GenericRouter<ObjCons, SF> {
    routes: HashMap<&'static str, SF>,
    _req_body: PhantomData<ObjCons>,
}

/// Error type of Router service.
pub type RouterError<E> = PipelineE<MatchError, E>;

impl<ObjCons, SF> Default for GenericRouter<ObjCons, SF> {
    fn default() -> Self {
        Self::new()
    }
}

impl<SF> GenericRouter<(), SF> {
    /// Creates a new router with the [default object constructor](DefaultObjectConstructor).
    pub fn with_default_object<Req, Arg>() -> GenericRouter<DefaultObjectConstructor<Req, Arg>, SF> {
        GenericRouter::new()
    }

    /// Creates a new router with a custom [object constructor](ObjectConstructor).
    pub fn with_custom_object<ObjCons>() -> GenericRouter<ObjCons, SF> {
        GenericRouter::new()
    }
}

impl<ObjCons, SF> GenericRouter<ObjCons, SF> {
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

impl<ObjCons, SF, Req, Arg> ServiceFactory<Req, Arg> for GenericRouter<ObjCons, SF>
where
    SF: ServiceFactory<Req, Arg>,
    Req: BorrowReq<http::Uri>,
    Arg: Clone,
{
    type Response = SF::Response;
    type Error = RouterError<SF::Error>;
    type Service = RouterService<SF::Service>;
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
                let service = fut.await.map_err(RouterError::Second)?;
                routes.insert(path, service).unwrap();
            }

            Ok(RouterService { routes })
        }
    }
}

pub struct RouterService<S> {
    routes: Node<S>,
}

impl<S, Req> Service<Req> for RouterService<S>
where
    S: Service<Req>,
    Req: BorrowReq<http::Uri>,
{
    type Response = S::Response;
    type Error = RouterError<S::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let service = self.routes.at(req.borrow().path()).map_err(RouterError::First)?;

            service.value.call(req).await.map_err(RouterError::Second)
        }
    }
}

impl<S, Req> ReadyService<Req> for RouterService<S>
where
    S: Service<Req>,
    Req: BorrowReq<http::Uri>,
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
