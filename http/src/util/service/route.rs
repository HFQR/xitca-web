use core::{fmt, future::Future};

use std::error;

use xitca_service::{pipeline::PipelineE, ready::ReadyService, Service};

use crate::http::{BorrowReq, Method};

mod next {
    pub struct Exist<S>(pub S);
    pub struct Empty;
}

macro_rules! method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<R>(route: R) -> Route<R, next::Empty, 1> {
            Route::new([Method::$method]).route(route)
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

pub struct Route<R, N, const M: usize> {
    methods: [Method; M],
    route: R,
    next: N,
}

impl<const N: usize> Route<(), next::Empty, N> {
    pub const fn new(methods: [Method; N]) -> Self {
        assert!(N > 0, "Route method can not be empty");
        Self {
            methods,
            route: (),
            next: next::Empty,
        }
    }

    pub fn route<R>(self, route: R) -> Route<R, next::Empty, N> {
        Route {
            methods: self.methods,
            route,
            next: self.next,
        }
    }
}

macro_rules! route_method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<R1>(self, $method_fn: R1) -> Route<R, next::Exist<Route<R1, N, 1>>, M> {
            self.next(Route::new([Method::$method]).route($method_fn))
        }
    };
}

impl<R, N, const M: usize> Route<R, N, M> {
    // TODO is this really the intended behavior? insert `next` between `self` and `self.next`?
    pub fn next<R1, const M1: usize>(
        self,
        next: Route<R1, next::Empty, M1>,
    ) -> Route<R, next::Exist<Route<R1, N, M1>>, M> {
        for m in next.methods.iter() {
            if self.methods.contains(m) {
                panic!("{m} method already exists. Route can not contain overlapping methods.");
            }
        }

        Route {
            methods: self.methods,
            route: self.route,
            next: next::Exist(Route {
                methods: next.methods,
                route: next.route,
                next: self.next,
            }),
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

impl<Arg, R, N, const M: usize> Service<Arg> for Route<R, next::Exist<N>, M>
where
    R: Service<Arg>,
    N: Service<Arg, Error = R::Error>,
    Arg: Clone,
{
    type Response = RouteService<R::Response, next::Exist<N::Response>, M>;
    type Error = R::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s>(&'s self, arg: Arg) -> Self::Future<'s>
    where
        Arg: 's,
    {
        async {
            let route = self.route.call(arg.clone()).await?;
            let next = self.next.0.call(arg).await?;
            Ok(RouteService {
                methods: self.methods.clone(),
                route,
                next: next::Exist(next),
            })
        }
    }
}

impl<Arg, R, const M: usize> Service<Arg> for Route<R, next::Empty, M>
where
    R: Service<Arg>,
{
    type Response = RouteService<R::Response, next::Empty, M>;
    type Error = R::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s>(&'s self, arg: Arg) -> Self::Future<'s>
    where
        Arg: 's,
    {
        async {
            let route = self.route.call(arg).await?;
            Ok(RouteService {
                methods: self.methods.clone(),
                route,
                next: next::Empty,
            })
        }
    }
}

pub struct RouteService<R, N, const M: usize> {
    methods: [Method; M],
    route: R,
    next: N,
}

impl<R, N, Req, E, const M: usize> Service<Req> for RouteService<R, next::Exist<N>, M>
where
    R: Service<Req, Error = E>,
    N: Service<Req, Response = R::Response, Error = RouteError<E>>,
    Req: BorrowReq<Method>,
{
    type Response = R::Response;
    type Error = RouteError<E>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        async {
            if self.methods.contains(req.borrow()) {
                self.route.call(req).await.map_err(RouteError::Second)
            } else {
                self.next
                    .0
                    .call(req)
                    .await
                    .map_err(|e| try_append_allowed(e, &self.methods))
            }
        }
    }
}

#[cold]
#[inline(never)]
fn try_append_allowed<E>(mut e: RouteError<E>, methods: &[Method]) -> RouteError<E> {
    if let RouteError::First(ref mut e) = e {
        e.0.extend_from_slice(methods);
    }
    e
}

impl<R, Req, const M: usize> Service<Req> for RouteService<R, next::Empty, M>
where
    R: Service<Req>,
    Req: BorrowReq<Method>,
{
    type Response = R::Response;
    type Error = RouteError<R::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        async {
            if self.methods.contains(req.borrow()) {
                self.route.call(req).await.map_err(RouteError::Second)
            } else {
                Err(RouteError::First(MethodNotAllowed(
                    self.methods.iter().cloned().collect(),
                )))
            }
        }
    }
}

impl<R, N, const M: usize> ReadyService for RouteService<R, N, M> {
    type Ready = ();
    type ReadyFuture<'f> = impl Future<Output = Self::Ready> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async {}
    }
}

/// Error type of Route service.
/// `First` variant contains [MethodNotAllowed] error.
/// `Second` variant contains error returned by the service passed to Route.
pub type RouteError<E> = PipelineE<MethodNotAllowed, E>;

/// Error type of Method not allow for route.
pub struct MethodNotAllowed(Vec<Method>);

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
        write!(f, "Method is not allowed")
    }
}

impl error::Error for MethodNotAllowed {}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{fn_service, Service, ServiceExt};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        body::{RequestBody, ResponseBody},
        http::Request,
        response::Response,
    };

    use super::*;

    async fn index(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Infallible> {
        Ok(Response::new(ResponseBody::None))
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
        assert!(matches!(err, RouteError::First(MethodNotAllowed(_))));
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
        assert!(matches!(err, RouteError::First(MethodNotAllowed(_))));

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

        let RouteError::First(e) = service.call(req).now_or_panic().err().unwrap() else {
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
