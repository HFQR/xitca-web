pub use xitca_router::{params::Params, MatchError};

use core::{future::Future, marker::PhantomData};

use std::{borrow::Cow, collections::HashMap};

use xitca_service::{
    object::{DefaultObjectConstructor, IntoObject},
    pipeline::PipelineE,
    ready::ReadyService,
    EnclosedFactory, EnclosedFnFactory, FnService, Service,
};

use crate::http::{BorrowReq, BorrowReqMut, Uri};

use super::route::Route;

/// Simple router for matching on [Request](crate::http::Request)'s path and call according service.
///
/// An [ObjectConstructor] must be specified as a type parameter
/// in order to determine how the router type-erases node services.
pub struct Router<SF, ObjCons = DefaultObjectConstructor> {
    routes: HashMap<Cow<'static, str>, SF>,
    _obj: PhantomData<ObjCons>,
}

/// Error type of Router service.
/// `First` variant contains [MatchError] error.
/// `Second` variant contains error returned by the services passed to Router.
pub type RouterError<E> = PipelineE<MatchError, E>;

impl<SF, ObjCons> Default for Router<SF, ObjCons> {
    fn default() -> Self {
        Router::with_custom_object()
    }
}

impl<SF> Router<SF> {
    pub fn new() -> Self {
        Router::with_custom_object()
    }

    /// Creates a new router with a custom [object constructor](ObjectConstructor).
    pub fn with_custom_object<ObjCons>() -> Router<SF, ObjCons> {
        Router {
            routes: HashMap::new(),
            _obj: PhantomData,
        }
    }
}

impl<SF, ObjCons> Router<SF, ObjCons> {
    /// Insert a new service factory to given path.
    ///
    /// # Panic:
    ///
    /// When multiple services inserted with the same path.
    pub fn insert<F, Arg, Req>(mut self, path: &'static str, mut factory: F) -> Self
    where
        F: PathGen,
        ObjCons: IntoObject<F, Arg, Req, Object = SF>,
    {
        let path = factory.gen(path);
        assert!(self.routes.insert(path, ObjCons::into_object(factory)).is_none());
        self
    }
}

/// trait for producing actual router path with given prefix str.
/// default to pass through (the router path is the same as prefix)
pub trait PathGen {
    fn gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        Cow::Borrowed(prefix)
    }
}

// nest router needs special handling for path generation.
impl<SF, ObjCons> PathGen for Router<SF, ObjCons> {
    fn gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        let mut path = String::from(prefix);
        if path.ends_with('/') {
            path.pop();
        }

        self.routes = self
            .routes
            .drain()
            .map(|(k, v)| {
                let mut path = path.clone();
                path.push_str(k.as_ref());
                (Cow::Owned(path), v)
            })
            .collect();

        path.push_str("/:r");

        Cow::Owned(path)
    }
}

impl<R, N, const M: usize> PathGen for Route<R, N, M> {}

impl<F> PathGen for FnService<F> {}

impl<F, S> PathGen for EnclosedFactory<F, S>
where
    F: PathGen,
{
    fn gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.first.gen(prefix)
    }
}

impl<F, S> PathGen for EnclosedFnFactory<F, S>
where
    F: PathGen,
{
    fn gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.first.gen(prefix)
    }
}

impl<SF, ObjCons, Arg> Service<Arg> for Router<SF, ObjCons>
where
    SF: Service<Arg>,
    Arg: Clone,
{
    type Response = RouterService<SF::Response>;
    type Error = SF::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s>(&'s self, arg: Arg) -> Self::Future<'s>
    where
        Arg: 's,
    {
        async move {
            let mut routes = xitca_router::Router::new();

            for (path, service) in self.routes.iter() {
                let service = service.call(arg.clone()).await?;
                routes.insert(path.to_string(), service).unwrap();
            }

            Ok(RouterService { routes })
        }
    }
}

pub struct RouterService<S> {
    routes: xitca_router::Router<S>,
}

impl<S, Req> Service<Req> for RouterService<S>
where
    S: Service<Req>,
    Req: BorrowReq<Uri> + BorrowReqMut<Params>,
{
    type Response = S::Response;
    type Error = RouterError<S::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, mut req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        async {
            let xitca_router::Match { value, params } =
                self.routes.at(req.borrow().path()).map_err(RouterError::First)?;

            *req.borrow_mut() = params;

            value.call(req).await.map_err(RouterError::Second)
        }
    }
}

impl<S> ReadyService for RouterService<S> {
    type Ready = ();
    type Future<'f> = impl Future<Output = Self::Ready> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::Future<'_> {
        async {}
    }
}

#[cfg(test)]
mod test {
    use core::convert::Infallible;

    use xitca_service::{fn_service, Service, ServiceExt};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::http::{Request, RequestExt, Response};

    use super::*;

    async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
    where
        S: Service<Req>,
    {
        service.call(req).await
    }

    #[test]
    fn router_accept_request() {
        Router::new()
            .insert(
                "/",
                fn_service(|_: Request<RequestExt<()>>| async { Ok::<_, Infallible>(Response::new(())) }),
            )
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();
    }

    #[test]
    fn router_enclosed_fn() {
        Router::new()
            .insert(
                "/",
                fn_service(|_: Request<RequestExt<()>>| async { Ok::<_, Infallible>(Response::new(())) }),
            )
            .enclosed_fn(enclosed)
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();
    }

    #[test]
    fn router_add_params_http() {
        async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
        where
            S: Service<Req>,
        {
            service.call(req).await
        }

        let req = Request::builder().uri("/users/1").body(Default::default()).unwrap();

        Router::new()
            .insert(
                "/users/:id",
                fn_service(|req: Request<RequestExt<()>>| async move {
                    let params = req.body().params();
                    assert_eq!(params.get("id").unwrap(), "1");
                    Ok::<_, Infallible>(Response::new(()))
                }),
            )
            .enclosed_fn(enclosed)
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .unwrap();
    }

    #[test]
    fn router_nest() {
        let router = || {
            Router::new()
                .insert(
                    "/nest",
                    fn_service(|_: Request<RequestExt<()>>| async { Ok::<_, Infallible>(Response::new(())) }),
                )
                .enclosed_fn(enclosed)
        };

        let req = || {
            Request::builder()
                .uri("http://foo.bar/scope/nest")
                .body(Default::default())
                .unwrap()
        };

        Router::new()
            .insert("/scope", router())
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req())
            .now_or_panic()
            .unwrap();

        Router::new()
            .insert("/scope/", router())
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req())
            .now_or_panic()
            .unwrap();
    }
}
