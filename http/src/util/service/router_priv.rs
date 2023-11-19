pub use xitca_router::{params::Params, MatchError};

use core::marker::PhantomData;

use std::{borrow::Cow, collections::HashMap};

use xitca_service::{
    object::BoxedServiceObject, pipeline::PipelineE, ready::ReadyService, EnclosedFactory, EnclosedFnFactory,
    FnService, Service,
};

use crate::http::{BorrowReq, BorrowReqMut, Request, Uri};

use super::route::Route;

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
    pub fn insert<F, Arg, Req>(mut self, path: &'static str, mut factory: F) -> Self
    where
        F: Service<Arg> + PathGen,
        F::Response: Service<Req>,
        Req: IntoObject<F, Arg, UnSyncMarker, Object = Obj>,
    {
        let path = factory.gen(path);
        assert!(self.routes.insert(path, Req::into_object(factory)).is_none());
        self
    }

    /// sync version of [Router::insert] where the service factory type must be a thread safe
    /// type that is bound to `Send` and `Sync` auto traits.
    pub fn insert_sync<F, Arg, Req>(mut self, path: &'static str, mut factory: F) -> Self
    where
        F: Service<Arg> + PathGen + Send + Sync,
        F::Response: Service<Req>,
        Req: IntoObject<F, Arg, SyncMarker, Object = Obj>,
    {
        let path = factory.gen(path);
        assert!(self.routes.insert(path, Req::into_object(factory)).is_none());
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
impl<Obj> PathGen for Router<Obj> {
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

impl<S, Req> Service<Req> for RouterService<S>
where
    S: xitca_service::object::ServiceObject<Req>,
    Req: BorrowReq<Uri> + BorrowReqMut<Params>,
{
    type Response = S::Response;
    type Error = RouterError<S::Error>;

    // as of the time of committing rust compiler have problem optimizing this piece of code.
    // using async fn call directly would cause significant code bloating.
    #[allow(clippy::manual_async_fn)]
    #[inline]
    fn call(&self, mut req: Req) -> impl core::future::Future<Output = Result<Self::Response, Self::Error>> {
        async {
            let xitca_router::Match { value, params } =
                self.routes.at(req.borrow().path()).map_err(RouterError::First)?;
            *req.borrow_mut() = params;
            xitca_service::object::ServiceObject::call(value, req)
                .await
                .map_err(RouterError::Second)
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
pub trait IntoObject<I, Arg, M>
where
    M: _seal::Marker,
{
    /// The type-erased form of `I`.
    type Object;

    /// Constructs `Self::Object` from `I`.
    fn into_object(inner: I) -> Self::Object;
}

/// object marker that bound Router's routes to Send and Sync.
pub struct SyncMarker;

// object marker that bound Router's routes to !Send and !Sync.
pub struct UnSyncMarker;

mod _seal {
    use super::{SyncMarker, UnSyncMarker};

    pub trait Marker {}

    impl Marker for SyncMarker {}
    impl Marker for UnSyncMarker {}
}

impl<T, Arg, Ext, Res, Err> IntoObject<T, Arg, UnSyncMarker> for Request<Ext>
where
    Ext: 'static,
    T: Service<Arg> + 'static,
    T::Response: Service<Request<Ext>, Response = Res, Error = Err> + 'static,
{
    type Object = BoxedServiceObject<Arg, BoxedServiceObject<Request<Ext>, Res, Err>, T::Error>;

    fn into_object(inner: T) -> Self::Object {
        Box::new(Builder(inner, PhantomData))
    }
}

impl<T, Arg, Ext, Res, Err> IntoObject<T, Arg, SyncMarker> for Request<Ext>
where
    Ext: 'static,
    T: Service<Arg> + Send + Sync + 'static,
    T::Response: Service<Request<Ext>, Response = Res, Error = Err> + 'static,
{
    type Object = Box<
        dyn xitca_service::object::ServiceObject<
                Arg,
                Response = BoxedServiceObject<Request<Ext>, Res, Err>,
                Error = T::Error,
            > + Send
            + Sync,
    >;

    fn into_object(inner: T) -> Self::Object {
        Box::new(Builder(inner, PhantomData))
    }
}

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
    fn router_sync() {
        fn bound_check<T>(_: T)
        where
            T: Send + Sync,
        {
        }

        bound_check(Router::new().insert_sync(
            "/",
            fn_service(|_: Request<RequestExt<()>>| async { Ok::<_, Infallible>(Response::new(())) }),
        ))
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
