pub use xitca_router::{params::Params, MatchError};

use core::{fmt, marker::PhantomData};

use std::{borrow::Cow, collections::HashMap, error};

use xitca_service::{
    object::{BoxedServiceObject, BoxedSyncServiceObject},
    pipeline::PipelineT,
    ready::ReadyService,
    FnService, Service,
};

use crate::http::{BorrowReq, BorrowReqMut, Request, Uri};

use super::{
    handler::HandlerService,
    route::{MethodNotAllowed, Route},
};

/// Simple router for matching path and call according service.
///
/// An [ServiceObject](xitca_service::object::ServiceObject) must be specified as a type parameter
/// in order to determine how the router type-erases node services.
pub struct Router<Obj> {
    routes: HashMap<Cow<'static, str>, Obj>,
}

/// Error type of Router service.
pub enum RouterError<E> {
    /// failed to match on a routed service.
    Match(MatchError),
    /// a match of service is found but it's not allowed for access.
    NotAllowed(MethodNotAllowed),
    /// error produced by routed service.
    Service(E),
}

impl<E> fmt::Debug for RouterError<E>
where
    E: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Match(ref e) => fmt::Debug::fmt(e, f),
            Self::NotAllowed(ref e) => fmt::Debug::fmt(e, f),
            Self::Service(ref e) => fmt::Debug::fmt(e, f),
        }
    }
}

impl<E> fmt::Display for RouterError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Match(ref e) => fmt::Display::fmt(e, f),
            Self::NotAllowed(ref e) => fmt::Display::fmt(e, f),
            Self::Service(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl<E> error::Error for RouterError<E> where E: error::Error {}

impl<Obj> Default for Router<Obj> {
    fn default() -> Self {
        Router::new()
    }
}

impl<Obj> Router<Obj> {
    pub fn new() -> Self {
        Router { routes: HashMap::new() }
    }
}

impl<Obj> Router<Obj> {
    /// Insert a new service builder to given path. The service builder must produce another
    /// service type that impl [Service] trait while it's generic `Req` type must impl
    /// [IntoObject] trait.
    ///
    /// # Panic:
    ///
    /// When multiple services inserted to the same path.
    pub fn insert<F, Arg, Req>(mut self, path: &'static str, mut builder: F) -> Self
    where
        F: Service<Arg> + RouterGen + Send + Sync,
        F::Response: Service<Req>,
        Req: IntoObject<F::Route<F>, Arg, Object = Obj>,
    {
        let path = builder.path_gen(path);
        assert!(self
            .routes
            .insert(path, Req::into_object(F::route_gen(builder)))
            .is_none());
        self
    }

    #[doc(hidden)]
    /// See [TypedRoute] for detail.
    pub fn insert_typed<T, M>(mut self, _: T) -> Router<Obj>
    where
        T: TypedRoute<M, Route = Obj>,
    {
        let path = T::path();
        let route = T::route();
        assert!(self.routes.insert(Cow::Borrowed(path), route).is_none());
        self
    }
}

/// trait for specialized route generation when utilizing [Router::insert].
pub trait RouterGen {
    /// service builder type for generating the final route service.
    type Route<R>;

    /// path generator.
    ///
    /// default to passthrough of original prefix path.
    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        Cow::Borrowed(prefix)
    }

    /// route service generator.
    ///
    /// implicit default to map error type to [RouterError] with [RouterMapErr].
    fn route_gen<R>(route: R) -> Self::Route<R>;
}

// nest router needs special handling for path generation.
impl<Obj> RouterGen for Router<Obj> {
    type Route<R> = R;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
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

        path.push_str("/*");

        Cow::Owned(path)
    }

    fn route_gen<R>(route: R) -> Self::Route<R> {
        route
    }
}

impl<R, N, const M: usize> RouterGen for Route<R, N, M> {
    type Route<R1> = R1;

    fn route_gen<R1>(route: R1) -> Self::Route<R1> {
        route
    }
}

impl<F, T, M> RouterGen for HandlerService<F, T, M> {
    type Route<R> = RouterMapErr<R>;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        RouterMapErr(route)
    }
}

impl<F> RouterGen for FnService<F> {
    type Route<R1> = RouterMapErr<R1>;

    fn route_gen<R1>(route: R1) -> Self::Route<R1> {
        RouterMapErr(route)
    }
}

impl<F, S, M> RouterGen for PipelineT<F, S, M>
where
    F: RouterGen,
{
    type Route<R> = F::Route<R>;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.first.path_gen(prefix)
    }

    fn route_gen<R>(route: R) -> Self::Route<R> {
        F::route_gen(route)
    }
}

/// default error mapper service that map all service error type to `RouterError::Second`
pub struct RouterMapErr<S>(pub S);

impl<S, Arg> Service<Arg> for RouterMapErr<S>
where
    S: Service<Arg>,
{
    type Response = MapErrService<S::Response>;
    type Error = S::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        self.0.call(arg).await.map(MapErrService)
    }
}

pub struct MapErrService<S>(S);

impl<S, Req> Service<Req> for MapErrService<S>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = RouterError<S::Error>;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.0.call(req).await.map_err(RouterError::Service)
    }
}

impl<Obj, Arg> Service<Arg> for Router<Obj>
where
    Obj: Service<Arg>,
    Arg: Clone,
{
    type Response = RouterService<Obj::Response>;
    type Error = Obj::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        let mut routes = xitca_router::Router::new();

        for (path, service) in self.routes.iter() {
            let service = service.call(arg.clone()).await?;
            routes.insert(path.to_string(), service).unwrap();
        }

        Ok(RouterService { routes })
    }
}

pub struct RouterService<S> {
    routes: xitca_router::Router<S>,
}

impl<S, Req, E> Service<Req> for RouterService<S>
where
    S: Service<Req, Error = RouterError<E>>,
    Req: BorrowReq<Uri> + BorrowReqMut<Params>,
{
    type Response = S::Response;
    type Error = S::Error;

    // as of the time of committing rust compiler have problem optimizing this piece of code.
    // using async fn call directly would cause significant code bloating.
    #[allow(clippy::manual_async_fn)]
    #[inline]
    fn call(&self, mut req: Req) -> impl core::future::Future<Output = Result<Self::Response, Self::Error>> {
        async {
            let xitca_router::Match { value, params } =
                self.routes.at(req.borrow().path()).map_err(RouterError::Match)?;
            *req.borrow_mut() = params;
            Service::call(value, req).await
        }
    }
}

impl<S> ReadyService for RouterService<S> {
    type Ready = ();

    #[inline]
    async fn ready(&self) -> Self::Ready {}
}

/// An object constructor represents a one of possibly many ways to create a trait object from `I`.
///
/// A [Service] type, for example, may be type-erased into `Box<dyn Service<&'static str>>`,
/// `Box<dyn for<'a> Service<&'a str>>`, `Box<dyn Service<&'static str> + Service<u8>>`, etc.
/// Each would be a separate impl for [IntoObject].
pub trait IntoObject<I, Arg> {
    /// The type-erased form of `I`.
    type Object;

    /// Constructs `Self::Object` from `I`.
    fn into_object(inner: I) -> Self::Object;
}

impl<T, Arg, Ext, Res, Err> IntoObject<T, Arg> for Request<Ext>
where
    Ext: 'static,
    T: Service<Arg> + Send + Sync + 'static,
    T::Response: Service<Request<Ext>, Response = Res, Error = Err> + 'static,
{
    type Object = BoxedSyncServiceObject<Arg, BoxedServiceObject<Request<Ext>, Res, Err>, T::Error>;

    fn into_object(inner: T) -> Self::Object {
        struct Builder<T, Req>(T, PhantomData<fn(Req)>);

        impl<T, Req, Arg, Res, Err> Service<Arg> for Builder<T, Req>
        where
            T: Service<Arg> + 'static,
            T::Response: Service<Req, Response = Res, Error = Err> + 'static,
        {
            type Response = BoxedServiceObject<Req, Res, Err>;
            type Error = T::Error;

            async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
                self.0.call(arg).await.map(|s| Box::new(s) as _)
            }
        }

        Box::new(Builder(inner, PhantomData))
    }
}

#[doc(hidden)]
/// trait for concrete typed Router and Routes.
/// all generic types must be known when implementing the trait and Router<Obj>
/// would infer it's generic types from it.
pub trait TypedRoute<M = ()> {
    /// typed route. in form of Box<dyn ServiceObject<_>>
    type Route;

    /// method for providing matching path of Self::Route.
    fn path() -> &'static str;

    /// method for generating typed route.
    fn route() -> Self::Route;
}

#[cfg(test)]
mod test {
    use core::convert::Infallible;

    use xitca_service::{fn_service, Service, ServiceExt};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        http::{Request, RequestExt, Response},
        util::service::route::get,
    };

    use super::*;

    async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
    where
        S: Service<Req>,
    {
        service.call(req).await
    }

    async fn func(_: Request<RequestExt<()>>) -> Result<Response<()>, Infallible> {
        Ok(Response::new(()))
    }

    #[test]
    fn router_sync() {
        fn bound_check<T: Send + Sync>(_: T) {}

        bound_check(Router::new().insert("/", fn_service(func)))
    }

    #[test]
    fn router_accept_request() {
        Router::new()
            .insert("/", fn_service(func))
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
            .insert("/", fn_service(func))
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
        let handler = || get(fn_service(func)).enclosed_fn(enclosed);

        let router = || Router::new().insert("/nest", handler()).enclosed_fn(enclosed);

        let req = || {
            Request::builder()
                .uri("http://foo.bar/scope/nest")
                .body(Default::default())
                .unwrap()
        };

        Router::new()
            .insert("/raw", fn_service(func))
            .insert("/root", handler())
            .insert("/scope", router())
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req())
            .now_or_panic()
            .unwrap();

        Router::new()
            .insert("/root", handler())
            .insert("/scope/", router())
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req())
            .now_or_panic()
            .unwrap();
    }

    #[test]
    fn router_service_call_size() {
        let service = Router::new()
            .insert("/", fn_service(func))
            .call(())
            .now_or_panic()
            .unwrap();

        println!(
            "router service ready call size: {:?}",
            core::mem::size_of_val(&service.ready())
        );

        println!(
            "router service call size: {:?}",
            core::mem::size_of_val(&service.call(Request::default()))
        );
    }

    #[test]
    fn router_typed() {
        type Req = Request<RequestExt<()>>;
        type Route = BoxedServiceObject<Req, Response<()>, RouterError<Infallible>>;
        type RouteObject = BoxedSyncServiceObject<(), Route, Infallible>;

        struct Index;

        impl TypedRoute for Index {
            type Route = RouteObject;

            fn path() -> &'static str {
                "/"
            }

            fn route() -> Self::Route {
                Req::into_object(RouterMapErr(xitca_service::fn_service(func)))
            }
        }

        struct V2;

        impl TypedRoute for V2 {
            type Route = RouteObject;

            fn path() -> &'static str {
                "/v2"
            }

            fn route() -> Self::Route {
                Req::into_object(RouterMapErr(xitca_service::fn_service(func)))
            }
        }

        Router::new()
            .insert_typed(Index)
            .insert_typed(V2)
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();
    }
}
