pub use xitca_router::{MatchError, params::Params};

use core::{fmt, marker::PhantomData};

use std::{collections::HashMap, error};

use xitca_service::{BoxFuture, FnService, Service, object::BoxedServiceObject, pipeline::PipelineT};

use crate::http::Request;

use super::{
    handler::HandlerService,
    route::{MethodNotAllowed, Route},
};

pub use self::object::RouteObject;

/// Simple router for matching path and call according service.
///
/// An [ServiceObject](xitca_service::object::ServiceObject) must be specified as a type parameter
/// in order to determine how the router type-erases node services.
pub struct Router<Obj> {
    // record for last time PathGen is called with certain route string prefix.
    prefix: Option<usize>,
    routes: HashMap<String, Obj>,
}

impl<Obj> Default for Router<Obj> {
    fn default() -> Self {
        Router::new()
    }
}

impl<Obj> Router<Obj> {
    pub fn new() -> Self {
        Router {
            prefix: None,
            routes: HashMap::new(),
        }
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
        F: Service<Arg> + RouteGen + Send + Sync,
        F::Response: Service<Req>,
        Req: IntoObject<F::Route<F>, Arg, Object = Obj>,
    {
        let path = builder.path_gen(path);
        assert!(
            self.routes
                .insert(path, Req::into_object(F::route_gen(builder)))
                .is_none()
        );
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
        assert!(self.routes.insert(String::from(path), route).is_none());
        self
    }
}

impl<Obj, Arg> Service<Arg> for Router<Obj>
where
    Obj: Service<Arg>,
    Arg: Clone,
{
    type Response = service::RouterService<Obj::Response>;
    type Error = Obj::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        let mut router = xitca_router::Router::new();

        for (path, service) in self.routes.iter() {
            let service = service.call(arg.clone()).await?;
            router.insert(path.to_string(), service).unwrap();
        }

        Ok(service::RouterService {
            prefix: self.prefix,
            router,
        })
    }
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

/// trait for specialized route generation when utilizing [Router::insert].
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not impl PathGen trait",
    label = "route must impl PathGen trait for specialized route path configuration",
    note = "consider add `impl PathGen for {Self}`"
)]
pub trait PathGen {
    /// path generator.
    ///
    /// default to passthrough of original prefix path.
    fn path_gen(&mut self, prefix: &str) -> String {
        String::from(prefix)
    }
}

/// trait for specialized route generation when utilizing [Router::insert].
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not impl RouteGen trait",
    label = "route must impl RouteGen trait for specialized route path and service configuration",
    note = "consider add `impl PathGen for {Self}` and `impl RouteGen for {Self}`"
)]
pub trait RouteGen: PathGen {
    /// service builder type for generating the final route service.
    type Route<R>;

    /// route service generator.
    ///
    /// implicit default to map error type to [RouterError] with [RouterMapErr].
    fn route_gen<R>(route: R) -> Self::Route<R>;
}

// nest router needs special handling for path generation.
impl<Obj> PathGen for Router<Obj>
where
    Obj: PathGen,
{
    fn path_gen(&mut self, path: &str) -> String {
        let mut path = String::from(path);
        if path.ends_with("/*") {
            path.pop();
        }

        if path.ends_with('/') {
            path.pop();
        }

        let prefix = self.prefix.get_or_insert(0);
        *prefix += path.len();

        self.routes.iter_mut().for_each(|(_, v)| {
            v.path_gen(path.as_str());
        });

        path.push_str("/*");

        path
    }
}

impl<Obj> RouteGen for Router<Obj>
where
    Obj: RouteGen,
{
    type Route<R> = R;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        route
    }
}

impl<R, N, const M: usize> PathGen for Route<R, N, M> {}

impl<R, N, const M: usize> RouteGen for Route<R, N, M> {
    type Route<R1> = R1;

    fn route_gen<R1>(route: R1) -> Self::Route<R1> {
        route
    }
}

impl<F, T, M> PathGen for HandlerService<F, T, M> {}

impl<F, T, M> RouteGen for HandlerService<F, T, M> {
    type Route<R> = RouterMapErr<R>;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        RouterMapErr(route)
    }
}

impl<F> PathGen for FnService<F> {}

impl<F> RouteGen for FnService<F> {
    type Route<R1> = RouterMapErr<R1>;

    fn route_gen<R1>(route: R1) -> Self::Route<R1> {
        RouterMapErr(route)
    }
}

impl<F, S, M> PathGen for PipelineT<F, S, M>
where
    F: PathGen,
{
    fn path_gen(&mut self, prefix: &str) -> String {
        self.first.path_gen(prefix)
    }
}

impl<F, S, M> RouteGen for PipelineT<F, S, M>
where
    F: RouteGen,
{
    type Route<R> = F::Route<R>;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        F::route_gen(route)
    }
}

/// default error mapper service that map all service error type to `RouterError::Second`
pub struct RouterMapErr<S>(pub S);

impl<S> PathGen for RouterMapErr<S>
where
    S: PathGen,
{
    fn path_gen(&mut self, prefix: &str) -> String {
        self.0.path_gen(prefix)
    }
}

impl<S> RouteGen for RouterMapErr<S>
where
    S: RouteGen,
{
    type Route<R> = S::Route<R>;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        S::route_gen(route)
    }
}

impl<S, Arg> Service<Arg> for RouterMapErr<S>
where
    S: Service<Arg>,
{
    type Response = service::MapErrService<S::Response>;
    type Error = S::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        self.0.call(arg).await.map(service::MapErrService)
    }
}

/// An object constructor represents a one of possibly many ways to create a trait object from `I`.
///
/// A [Service] type, for example, may be type-erased into `Box<dyn Service<&'static str>>`,
/// `Box<dyn for<'a> Service<&'a str>>`, `Box<dyn Service<&'static str> + Service<u8>>`, etc.
/// Each would be a separate impl for [IntoObject].
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not impl IntoObject trait",
    label = "route must impl IntoObject trait for specialized type erased service type",
    note = "consider add `impl IntoObject<_> for {Self}`"
)]
pub trait IntoObject<I, Arg> {
    /// The type-erased form of `I`.
    type Object;

    /// Constructs `Self::Object` from `I`.
    fn into_object(inner: I) -> Self::Object;
}

mod object {
    use super::*;

    pub trait RouteService<Arg>: PathGen {
        type Response;
        type Error;

        fn call<'s>(&'s self, arg: Arg) -> BoxFuture<'s, Self::Response, Self::Error>
        where
            Arg: 's;
    }

    impl<Arg, S> RouteService<Arg> for S
    where
        S: Service<Arg> + PathGen,
    {
        type Response = S::Response;
        type Error = S::Error;

        fn call<'s>(&'s self, arg: Arg) -> BoxFuture<'s, Self::Response, Self::Error>
        where
            Arg: 's,
        {
            Box::pin(Service::call(self, arg))
        }
    }

    pub struct RouteObject<Arg, S, E>(pub Box<dyn RouteService<Arg, Response = S, Error = E> + Send + Sync>);

    impl<Arg, S, E> PathGen for RouteObject<Arg, S, E> {
        fn path_gen(&mut self, prefix: &str) -> String {
            self.0.path_gen(prefix)
        }
    }

    impl<Arg, S, E> RouteGen for RouteObject<Arg, S, E> {
        type Route<R> = R;

        fn route_gen<R>(route: R) -> Self::Route<R> {
            route
        }
    }

    impl<Arg, S, E> Service<Arg> for RouteObject<Arg, S, E> {
        type Response = S;
        type Error = E;

        async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
            self.0.call(arg).await
        }
    }
}

impl<T, Arg, Ext, Res, Err> IntoObject<T, Arg> for Request<Ext>
where
    Ext: 'static,
    T: Service<Arg> + RouteGen + Send + Sync + 'static,
    T::Response: Service<Request<Ext>, Response = Res, Error = Err> + 'static,
{
    type Object = object::RouteObject<Arg, BoxedServiceObject<Request<Ext>, Res, Err>, T::Error>;

    fn into_object(inner: T) -> Self::Object {
        struct Builder<T, Req>(T, PhantomData<fn(Req)>);

        impl<T, Req> PathGen for Builder<T, Req>
        where
            T: PathGen,
        {
            fn path_gen(&mut self, prefix: &str) -> String {
                self.0.path_gen(prefix)
            }
        }

        impl<T, Req> RouteGen for Builder<T, Req>
        where
            T: RouteGen,
        {
            type Route<R> = T::Route<R>;

            fn route_gen<R>(route: R) -> Self::Route<R> {
                T::route_gen(route)
            }
        }

        impl<T, Req, Arg, Res, Err> Service<Arg> for Builder<T, Req>
        where
            T: Service<Arg> + RouteGen + 'static,
            T::Response: Service<Req, Response = Res, Error = Err> + 'static,
        {
            type Response = BoxedServiceObject<Req, Res, Err>;
            type Error = T::Error;

            async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
                self.0.call(arg).await.map(|s| Box::new(s) as _)
            }
        }

        object::RouteObject(Box::new(Builder(inner, PhantomData)))
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

mod service {
    use xitca_service::ready::ReadyService;

    use crate::http::{BorrowReq, BorrowReqMut, Uri};

    use super::{Params, RouterError, Service};

    pub struct RouterService<S> {
        // a length record of prefix of current router.
        // when it's Some the request path has to be sliced to exclude the string path prefix.
        pub(super) prefix: Option<usize>,
        pub(super) router: xitca_router::Router<S>,
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
                let mut path = req.borrow().path();

                if let Some(prefix) = self.prefix {
                    path = &path[prefix..];
                }

                let xitca_router::Match { value, params } = self.router.at(path).map_err(RouterError::Match)?;
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

    pub struct MapErrService<S>(pub(super) S);

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
}

#[cfg(test)]
mod test {
    use core::convert::Infallible;

    use xitca_service::{ServiceExt, fn_service, ready::ReadyService};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        http::{RequestExt, Response},
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

        Router::new()
            .insert(
                "/1111/",
                Router::new().insert(
                    "/222222/3",
                    Router::new().insert("/3333333", Router::new().insert("/4444", router())),
                ),
            )
            .call(())
            .now_or_panic()
            .unwrap()
            .call(
                Request::builder()
                    .uri("http://foo.bar/1111/222222/3/3333333/4444/nest")
                    .body(Default::default())
                    .unwrap(),
            )
            .now_or_panic()
            .unwrap();

        Router::new()
            .insert("/api", Router::new().insert("/v2", router()))
            .call(())
            .now_or_panic()
            .unwrap()
            .call(
                Request::builder()
                    .uri("http://foo.bar/api/v2/nest")
                    .body(Default::default())
                    .unwrap(),
            )
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
        type Object = object::RouteObject<(), Route, Infallible>;

        struct Index;

        impl TypedRoute for Index {
            type Route = Object;

            fn path() -> &'static str {
                "/"
            }

            fn route() -> Self::Route {
                Req::into_object(RouterMapErr(xitca_service::fn_service(func)))
            }
        }

        struct V2;

        impl TypedRoute for V2 {
            type Route = Object;

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
