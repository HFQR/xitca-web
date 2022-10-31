use std::{
    collections::HashMap,
    error,
    fmt::{self, Debug, Display, Formatter},
    future::Future,
    marker::PhantomData,
};

use xitca_service::{
    object::{DefaultObject, DefaultObjectConstructor, ObjectConstructor},
    pipeline::PipelineE,
    ready::ReadyService,
    Service,
};

use crate::{http, request::BorrowReq};

/// A [GenericRouter] specialized with [DefaultObjectConstructor]
pub type Router<Req, Arg, BErr, Res, Err> =
    GenericRouter<DefaultObjectConstructor<Req, Arg>, DefaultObject<Arg, Req, BErr, Res, Err>>;

/// Simple router for matching on [Request]'s path and call according service.
///
/// An [ObjectConstructor] must be specified as a type prameter
/// in order to determine how the router type-erases node services.
pub struct GenericRouter<ObjCons, SF> {
    routes: HashMap<&'static str, SF>,
    _req_body: PhantomData<ObjCons>,
}

/// Error type of Router service.
/// `First` variant contains [MatchError] error.
/// `Second` variant contains error returned by the services passed to Router.
pub type RouterError<E> = PipelineE<MatchError, E>;

/// Error for request failed to match on services inside Router.
pub struct MatchError {
    inner: matchit::MatchError,
}

impl MatchError {
    /// Indicates whether a route exists at the same path with/without a trailing slash.
    pub fn is_trailing_slash(&self) -> bool {
        matches!(
            self.inner,
            matchit::MatchError::MissingTrailingSlash | matchit::MatchError::ExtraTrailingSlash
        )
    }
}

impl Debug for MatchError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.inner, f)
    }
}

impl Display for MatchError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.inner, f)
    }
}

impl error::Error for MatchError {}

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

impl<ObjCons, SF, Arg> Service<Arg> for GenericRouter<ObjCons, SF>
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
            let mut routes = matchit::Router::new();

            for (path, service) in self.routes.iter() {
                let service = service.call(arg.clone()).await?;
                routes.insert(*path, service).unwrap();
            }

            Ok(RouterService { routes })
        }
    }
}

pub struct RouterService<S> {
    routes: matchit::Router<S>,
}

impl<S, Req> Service<Req> for RouterService<S>
where
    S: Service<Req>,
    Req: BorrowReq<http::Uri>,
{
    type Response = S::Response;
    type Error = RouterError<S::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        async {
            let service = self
                .routes
                .at(req.borrow().path())
                .map_err(|inner| RouterError::First(MatchError { inner }))?;

            service.value.call(req).await.map_err(RouterError::Second)
        }
    }
}

impl<S> ReadyService for RouterService<S> {
    type Ready = ();
    type ReadyFuture<'f> = impl Future<Output = Self::Ready> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async {}
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{fn_service, Service, ServiceExt};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{http, request::Request, response::Response};

    use super::*;

    #[test]
    fn router_accept_crate_request() {
        Router::new()
            .insert(
                "/",
                fn_service(|_: Request<()>| async { Ok::<_, Infallible>(Response::new(())) }),
            )
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::new(()))
            .now_or_panic()
            .unwrap();
    }

    // TODO: this test is a demenstration of possible lazliy populated liftime field of input Request type.
    // When the syntax is stable this implementation should be added to various service types.
    #[test]
    fn router_accept_non_static_request() {
        pub struct Request<'a> {
            uri: http::Uri,
            path: Option<&'a str>,
        }

        impl BorrowReq<http::Uri> for Request<'_> {
            fn borrow(&self) -> &http::Uri {
                &self.uri
            }
        }

        let req = Request {
            uri: http::Uri::from_static("/"),
            path: None,
        };

        struct MutatePath;

        impl<S> Service<S> for MutatePath {
            type Response = MutatePathService<S>;
            type Error = Infallible;
            type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f;

            fn call<'s>(&'s self, service: S) -> Self::Future<'s>
            where
                S: 's,
            {
                async {
                    Ok(MutatePathService {
                        service,
                        path: String::from("test"),
                    })
                }
            }
        }

        struct MutatePathService<S> {
            service: S,
            path: String,
        }

        impl<'r, S, Res, Err> Service<Request<'r>> for MutatePathService<S>
        where
            S: for<'r2> Service<Request<'r2>, Response = Res, Error = Err>,
        {
            type Response = Res;
            type Error = Err;
            type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f, 'r: 'f;

            fn call<'s>(&'s self, mut req: Request<'r>) -> Self::Future<'s>
            where
                'r: 's,
            {
                async move {
                    req.path = Some(self.path.as_str());
                    self.service.call(req).await
                }
            }
        }

        use std::boxed::Box;

        use xitca_service::{
            object::{Object, ObjectConstructor, ServiceObject, Wrapper},
            Service,
        };

        pub struct NonStaticObj;

        pub type TestServiceAlias<Res, Err> = impl for<'r> Service<Request<'r>, Response = Res, Error = Err>;

        impl<I, Svc, BErr, Res, Err> ObjectConstructor<I> for NonStaticObj
        where
            I: Service<Response = Svc, Error = BErr> + 'static,
            Svc: for<'r> Service<Request<'r>, Response = Res, Error = Err> + 'static,
        {
            type Object = Object<(), TestServiceAlias<Res, Err>, BErr>;

            fn into_object(inner: I) -> Self::Object {
                pub struct Obj<I>(I);

                impl<I, Svc, BErr, Res, Err> Service for Obj<I>
                where
                    I: Service<Response = Svc, Error = BErr> + 'static,
                    Svc: for<'r> Service<Request<'r>, Response = Res, Error = Err> + 'static,
                {
                    type Response = Wrapper<Box<dyn for<'r> ServiceObject<Request<'r>, Response = Res, Error = Err>>>;
                    type Error = BErr;
                    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f;

                    fn call<'s>(&'s self, arg: ()) -> Self::Future<'s>
                    where
                        (): 's,
                    {
                        async move {
                            let service = self.0.call(arg).await?;
                            Ok(Wrapper(Box::new(service) as _))
                        }
                    }
                }

                Object::from_service(Obj(inner))
            }
        }

        async fn handler(req: Request<'_>) -> Result<Response<()>, Infallible> {
            assert_eq!(req.path.as_deref(), Some("test"));
            Ok(Response::new(()))
        }

        let router = GenericRouter::with_custom_object::<NonStaticObj>()
            .insert("/", fn_service(handler))
            .enclosed(MutatePath);

        let service = Service::call(&router, ()).now_or_panic().unwrap();
        Service::call(&service, req).now_or_panic().unwrap();
    }

    #[test]
    fn router_accept_http_request() {
        Router::new()
            .insert(
                "/",
                fn_service(|_: http::Request<()>| async { Ok::<_, Infallible>(Response::new(())) }),
            )
            .call(())
            .now_or_panic()
            .unwrap()
            .call(http::Request::new(()))
            .now_or_panic()
            .unwrap();
    }

    #[test]
    fn router_enclosed_fn() {
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
            .call(())
            .now_or_panic()
            .unwrap()
            .call(http::Request::new(()))
            .now_or_panic()
            .unwrap();
    }
}
