//! routing [`Service`] with given [`Method`] that support a wide range of http method including custom ones.

use core::{fmt, marker::PhantomData};

use std::error;

use xitca_service::{Service, ready::ReadyService};

use crate::http::{BorrowReq, Method};

use super::router::RouterError;

macro_rules! method {
    ($method_fn: ident, $method: ident) => {
        #[doc = concat!("routing given [`Service`] type with [`Method::",stringify!($method),"`].")]
        /// Act as shortcut of [`Route::new`] and [`Route::route`].
        pub const fn $method_fn<R>(route: R) -> Route<R, MethodNotAllowedBuilder<R>, 1> {
            Route::_new([Method::$method], route)
        }
    };
}

method!(get, GET);
method!(post, POST);
method!(put, PUT);
method!(delete, DELETE);
method!(head, HEAD);
method!(options, OPTIONS);
method!(connect, CONNECT);
method!(patch, PATCH);
method!(trace, TRACE);

/// a tree type able of routing multiple [Method] against multiple [Service] types in linear manner.
pub struct Route<R, N, const M: usize> {
    methods: [Method; M],
    route: R,
    next: N,
}

type DefaultRoute<R, const M: usize> = Route<R, MethodNotAllowedBuilder<R>, M>;

impl<const M: usize> DefaultRoute<(), M> {
    /// construct a new Route with given array of methods.
    ///
    /// # Panics
    /// panic when input array is zero length.
    pub const fn new(methods: [Method; M]) -> Self {
        assert!(M > 0, "Route method can not be empty");
        Self::_new(methods, ())
    }

    /// pass certain [Service] type to Route so it can be matched against the
    /// methods Route contains.
    pub fn route<R>(self, route: R) -> DefaultRoute<R, M> {
        Route::_new(self.methods, route)
    }

    const fn _new<R>(methods: [Method; M], route: R) -> DefaultRoute<R, M> {
        Route {
            methods,
            route,
            next: MethodNotAllowedBuilder::new(),
        }
    }
}

macro_rules! route_method {
    ($method_fn: ident, $method: ident) => {
        #[doc = concat!("appending [Method::",stringify!($method),"] guarded route to current Route.")]
        /// Act as a shortcut of [Route::next].
        pub fn $method_fn<R1>(self, $method_fn: R1) -> Route<R, Route<R1, N, 1>, M> {
            self.next(Route::_new([Method::$method], $method_fn))
        }
    };
}

impl<R, N, const M: usize> Route<R, N, M> {
    /// append another Route to existing Route type.
    ///
    /// # Panics
    ///
    /// panic when chained Routes contain overlapping [Method]. Route only do liner method matching
    /// and overlapped method(s) will always enter the first Route matched against.
    pub fn next<R1, const M1: usize>(self, next: DefaultRoute<R1, M1>) -> Route<R, Route<R1, N, M1>, M> {
        for m in next.methods.iter() {
            if self.methods.contains(m) {
                panic!("{m} method already exists. Route can not contain overlapping methods.");
            }
        }

        // TODO is this really the intended behavior? insert `next` between `self` and `self.next`?
        Route {
            methods: self.methods,
            route: self.route,
            next: Route {
                methods: next.methods,
                route: next.route,
                next: self.next,
            },
        }
    }

    route_method!(get, GET);
    route_method!(post, POST);
    route_method!(put, PUT);
    route_method!(delete, DELETE);
    route_method!(head, HEAD);
    route_method!(options, OPTIONS);
    route_method!(connect, CONNECT);
    route_method!(patch, PATCH);
    route_method!(trace, TRACE);
}

impl<Arg, R, N, const M: usize> Service<Arg> for Route<R, N, M>
where
    R: Service<Arg>,
    N: Service<Arg, Error = R::Error>,
    Arg: Clone,
{
    type Response = RouteService<R::Response, N::Response, M>;
    type Error = R::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        let route = self.route.call(arg.clone()).await?;
        let next = self.next.call(arg).await?;
        Ok(RouteService {
            methods: self.methods.clone(),
            route,
            next,
        })
    }
}

pub struct RouteService<R, N, const M: usize> {
    methods: [Method; M],
    route: R,
    next: N,
}

impl<R, N, Req, E, const M: usize> Service<Req> for RouteService<R, N, M>
where
    R: Service<Req, Error = E>,
    N: Service<Req, Response = R::Response, Error = RouterError<E>>,
    Req: BorrowReq<Method>,
{
    type Response = R::Response;
    type Error = RouterError<E>;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        if self.methods.contains(req.borrow()) {
            self.route.call(req).await.map_err(RouterError::Service)
        } else {
            self.next
                .call(req)
                .await
                .map_err(|e| try_append_allowed(e, &self.methods))
        }
    }
}

#[cold]
#[inline(never)]
fn try_append_allowed<E>(mut e: RouterError<E>, methods: &[Method]) -> RouterError<E> {
    if let RouterError::NotAllowed(ref mut e) = e {
        e.0.extend_from_slice(methods);
    }
    e
}

impl<R, N, const M: usize> ReadyService for RouteService<R, N, M> {
    type Ready = ();

    #[inline]
    async fn ready(&self) -> Self::Ready {}
}

/// Error type of Method not allow for route.
pub struct MethodNotAllowed(pub Box<Vec<Method>>);

impl MethodNotAllowed {
    /// slice of allowed methods of current route.
    pub fn allowed_methods(&self) -> &[Method] {
        &self.0
    }
}

impl fmt::Debug for MethodNotAllowed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodNotAllowed").finish()
    }
}

impl fmt::Display for MethodNotAllowed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("router error: method is not allowed")
    }
}

impl error::Error for MethodNotAllowed {}

pub struct MethodNotAllowedBuilder<R>(PhantomData<fn(R)>);

impl<R> MethodNotAllowedBuilder<R> {
    const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<Arg, R> Service<Arg> for MethodNotAllowedBuilder<R>
where
    R: Service<Arg>,
{
    type Response = MethodNotAllowedService<R::Response>;
    type Error = R::Error;

    async fn call(&self, _: Arg) -> Result<Self::Response, Self::Error> {
        Ok(MethodNotAllowedService(PhantomData))
    }
}

pub struct MethodNotAllowedService<R>(PhantomData<fn(R)>);

impl<R, Req> Service<Req> for MethodNotAllowedService<R>
where
    R: Service<Req>,
{
    type Response = R::Response;
    type Error = RouterError<R::Error>;

    async fn call(&self, _: Req) -> Result<Self::Response, Self::Error> {
        Err(RouterError::NotAllowed(MethodNotAllowed(Box::default())))
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{ServiceExt, fn_service};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        body::{RequestBody, ResponseBody},
        http::{Request, Response},
    };

    use super::*;

    async fn index(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Infallible> {
        Ok(Response::new(ResponseBody::none()))
    }

    #[test]
    fn route_enclosed_fn2() {
        async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
        where
            S: Service<Req>,
        {
            service.call(req).await
        }

        let route = get(fn_service(index))
            .post(fn_service(index))
            .trace(fn_service(index))
            .enclosed_fn(enclosed);

        let service = route.call(()).now_or_panic().ok().unwrap();
        let req = Request::new(RequestBody::None);
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::POST;
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::PUT;
        let err = service.call(req).now_or_panic().err().unwrap();
        assert!(matches!(err, RouterError::NotAllowed(_)));
    }

    #[test]
    fn route_mixed() {
        let route = get(fn_service(index)).next(Route::new([Method::POST, Method::PUT]).route(fn_service(index)));

        let service = route.call(()).now_or_panic().ok().unwrap();
        let req = Request::new(RequestBody::None);
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::POST;
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::DELETE;
        let err = service.call(req).now_or_panic().err().unwrap();
        assert!(matches!(err, RouterError::NotAllowed(_)));

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::PUT;
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }

    #[test]
    #[should_panic]
    fn nest_route_panic() {
        let _ = Route::new([Method::POST, Method::GET])
            .route(fn_service(index))
            .next(get(fn_service(index)));
    }

    #[test]
    #[should_panic]
    fn empty_route_panic() {
        let _ = Route::new([]).route(fn_service(index));
    }

    #[test]
    fn route_struct() {
        let route = Route::new([Method::POST, Method::PUT])
            .route(fn_service(index))
            .next(Route::new([Method::GET]).route(fn_service(index)))
            .next(Route::new([Method::OPTIONS]).route(fn_service(index)))
            .next(trace(fn_service(index)));

        let service = route.call(()).now_or_panic().ok().unwrap();
        let req = Request::new(RequestBody::None);
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::POST;
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::DELETE;

        let RouterError::NotAllowed(e) = service.call(req).now_or_panic().err().unwrap() else {
            panic!("route does not return error on unallowed method request");
        };

        let allowed = e.allowed_methods();

        assert_eq!(allowed.len(), 5);
        // strict allowed method order does not matter.
        // as long as the test can produce deterministic prediction it's fine.
        assert_eq!(allowed[0], Method::GET);
        assert_eq!(allowed[1], Method::OPTIONS);
        assert_eq!(allowed[2], Method::TRACE);
        assert_eq!(allowed[3], Method::POST);
        assert_eq!(allowed[4], Method::PUT);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::PUT;
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::TRACE;
        let res = service.call(req).now_or_panic().ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }

    #[test]
    fn route_accept_crate_request() {
        get(fn_service(|_: Request<()>| async {
            Ok::<_, Infallible>(Response::new(()))
        }))
        .call(())
        .now_or_panic()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();
    }
}
