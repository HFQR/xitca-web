pub use xitca_router::{params::Params, MatchError};

use core::marker::PhantomData;

use std::{borrow::Cow, collections::HashMap};

use xitca_service::{
    object::{BoxedServiceObject, BoxedSyncServiceObject},
    pipeline::PipelineE,
    ready::ReadyService,
    EnclosedFactory, EnclosedFnFactory, FnService, MapErrorServiceFactory, Service,
};

use crate::http::{BorrowReq, BorrowReqMut, Request, Uri};

use super::{handler::HandlerService, route::Route};

/// Simple router for matching path and call according service.
///
/// An [ServiceObject](xitca_service::object::ServiceObject) must be specified as a type parameter
/// in order to determine how the router type-erases node services.
pub struct Router<Obj> {
    routes: HashMap<Cow<'static, str>, Obj>,
}

/// Error type of Router service.
/// `First` variant contains [MatchError] error.
/// `Second` variant contains error returned by the services passed to Router.
pub type RouterError<E> = PipelineE<MatchError, E>;

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
        Req: IntoObject<F::ErrGen<F>, Arg, Object = Obj>,
    {
        let path = builder.path_gen(path);
        assert!(self
            .routes
            .insert(path, Req::into_object(F::err_gen(builder)))
            .is_none());
        self
    }
}

/// trait for specialized route generation when utilizing [Router::insert].
pub trait RouterGen {
    /// service builder type for generating according error type of router service.
    type ErrGen<R>;

    /// path generator.
    ///
    /// default to passthrough of original prefix path.
    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        Cow::Borrowed(prefix)
    }

    /// error generator.
    ///
    /// implicit default to map error type to [RouterError] with [RouterMapErr].
    fn err_gen<R>(route: R) -> Self::ErrGen<R>;
}

// nest router needs special handling for path generation.
impl<Obj> RouterGen for Router<Obj> {
    type ErrGen<R> = R;

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

        path.push_str("/:r");

        Cow::Owned(path)
    }

    fn err_gen<R>(route: R) -> Self::ErrGen<R> {
        route
    }
}

impl<R, N, const M: usize> RouterGen for Route<R, N, M> {
    type ErrGen<R1> = RouterMapErr<R1>;

    fn err_gen<R1>(route: R1) -> Self::ErrGen<R1> {
        RouterMapErr(route)
    }
}

impl<F, T, O, M> RouterGen for HandlerService<F, T, O, M> {
    type ErrGen<R> = RouterMapErr<R>;

    fn err_gen<R>(route: R) -> Self::ErrGen<R> {
        RouterMapErr(route)
    }
}

impl<F> RouterGen for FnService<F> {
    type ErrGen<R1> = RouterMapErr<R1>;

    fn err_gen<R1>(route: R1) -> Self::ErrGen<R1> {
        RouterMapErr(route)
    }
}

impl<F, S> RouterGen for MapErrorServiceFactory<F, S>
where
    F: RouterGen,
{
    type ErrGen<R> = F::ErrGen<R>;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.first.path_gen(prefix)
    }

    fn err_gen<R>(route: R) -> Self::ErrGen<R> {
        F::err_gen(route)
    }
}

impl<F, S> RouterGen for EnclosedFactory<F, S>
where
    F: RouterGen,
{
    type ErrGen<R> = F::ErrGen<R>;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.first.path_gen(prefix)
    }

    fn err_gen<R>(route: R) -> Self::ErrGen<R> {
        F::err_gen(route)
    }
}

impl<F, S> RouterGen for EnclosedFnFactory<F, S>
where
    F: RouterGen,
{
    type ErrGen<R> = F::ErrGen<R>;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.first.path_gen(prefix)
    }

    fn err_gen<R>(route: R) -> Self::ErrGen<R> {
        F::err_gen(route)
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
        self.0.call(req).await.map_err(RouterError::Second)
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
    S: xitca_service::object::ServiceObject<Req, Error = RouterError<E>>,
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
                self.routes.at(req.borrow().path()).map_err(RouterError::First)?;
            *req.borrow_mut() = params;
            xitca_service::object::ServiceObject::call(value, req).await
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
}
