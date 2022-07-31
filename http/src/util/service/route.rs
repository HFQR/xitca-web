use std::{
    error,
    fmt::{self, Debug, Display, Formatter},
    future::Future,
};

use xitca_service::{pipeline::PipelineE, ready::ReadyService, BuildService, Service};

use crate::{
    http::{self, Method},
    request::BorrowReq,
};

mod next {
    pub struct Exist<S>(pub S);
    pub struct Empty;
}

macro_rules! method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<R>(route: R) -> Route<R, next::Empty, 1> {
            Route::new(route).methods([Method::$method])
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

impl Route<(), (), 0> {
    pub fn new<R>(route: R) -> Route<R, next::Empty, 0> {
        Route {
            methods: [],
            route,
            next: next::Empty,
        }
    }
}

macro_rules! route_method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<R1>(self, $method_fn: R1) -> Route<R, next::Exist<Route<R1, N, 1>>, M> {
            self.next(Route::new($method_fn).methods([Method::$method]))
        }
    };
}

impl<R, N, const M: usize> Route<R, N, M> {
    pub fn methods<const M1: usize>(self, methods: [Method; M1]) -> Route<R, N, M1> {
        assert!(M1 > 0, "Route method can not be empty");

        if M != 0 {
            panic!(
                "Trying to overwrite existing {:?} methods with {:?}",
                self.methods, methods
            );
        }

        Route {
            methods,
            route: self.route,
            next: self.next,
        }
    }

    // TODO is this really the intended behavior? insert `next` between `self` and `self.next`?
    pub fn next<R1, const M1: usize>(
        self,
        next: Route<R1, next::Empty, M1>,
    ) -> Route<R, next::Exist<Route<R1, N, M1>>, M> {
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

impl<Arg, R, N, const M: usize> BuildService<Arg> for Route<R, next::Exist<N>, M>
where
    R: BuildService<Arg>,
    N: BuildService<Arg, Error = R::Error>,
    Arg: Clone,
{
    type Service = Route<R::Service, next::Exist<N::Service>, M>;
    type Error = R::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let route = self.route.build(arg.clone());
        let next = self.next.0.build(arg);

        let methods = self.methods.clone();

        async move {
            let route = route.await?;
            let next = next::Exist(next.await?);
            // re-use Route for Service trait type.
            Ok(Route { methods, route, next })
        }
    }
}

impl<Arg, R, const M: usize> BuildService<Arg> for Route<R, next::Empty, M>
where
    R: BuildService<Arg>,
{
    type Service = Route<R::Service, next::Empty, M>;
    type Error = R::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let route = self.route.build(arg);
        let methods = self.methods.clone();

        async move {
            let route = route.await?;
            let next = next::Empty;
            // re-use Route for Service trait type.
            Ok(Route { methods, route, next })
        }
    }
}

impl<Req, R, N, E, const M: usize> Service<Req> for Route<R, next::Exist<N>, M>
where
    R: Service<Req, Error = E>,
    N: Service<Req, Response = R::Response, Error = RouteError<E>>,
    Req: BorrowReq<http::Method>,
{
    type Response = R::Response;
    type Error = RouteError<E>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            if self.methods.contains(req.borrow()) {
                self.route.call(req).await.map_err(RouteError::Second)
            } else {
                self.next.0.call(req).await
            }
        }
    }
}

impl<Req, R, const M: usize> Service<Req> for Route<R, next::Empty, M>
where
    R: Service<Req>,
    Req: BorrowReq<http::Method>,
{
    type Response = R::Response;
    type Error = RouteError<R::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            if self.methods.contains(req.borrow()) {
                self.route.call(req).await.map_err(RouteError::Second)
            } else {
                Err(RouteError::First(MethodNotAllowed))
            }
        }
    }
}

impl<Req, R, N, const M: usize> ReadyService<Req> for Route<R, N, M>
where
    Self: Service<Req>,
{
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
pub struct MethodNotAllowed;

impl Debug for MethodNotAllowed {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodNotAllowed").finish()
    }
}

impl Display for MethodNotAllowed {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Method is not allowed")
    }
}

impl error::Error for MethodNotAllowed {}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{fn_service, BuildServiceExt, Service};

    use crate::{
        body::{RequestBody, ResponseBody},
        http,
        request::Request,
        response::Response,
    };

    use super::*;

    async fn index(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Infallible> {
        Ok(Response::new(ResponseBody::None))
    }

    #[tokio::test]
    async fn route_enclosed_fn2() {
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

        let service = route.build(()).await.ok().unwrap();
        let req = Request::new(RequestBody::None);
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::POST;
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::PUT;
        let err = service.call(req).await.err().unwrap();
        assert!(matches!(err, RouteError::First(MethodNotAllowed)));
    }

    #[tokio::test]
    async fn route_mixed() {
        let route = get(fn_service(index)).next(Route::new(fn_service(index)).methods([Method::POST, Method::PUT]));

        let service = route.build(()).await.ok().unwrap();
        let req = Request::new(RequestBody::None);
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::POST;
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::DELETE;
        let err = service.call(req).await.err().unwrap();
        assert!(matches!(err, RouteError::First(MethodNotAllowed)));

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::PUT;
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn route_struct() {
        let route = Route::new(fn_service(index))
            .methods([Method::POST, Method::PUT])
            .next(Route::new(fn_service(index)).methods([Method::GET]))
            .next(Route::new(fn_service(index)).methods([Method::OPTIONS]))
            .next(trace(fn_service(index)));

        let service = route.build(()).await.ok().unwrap();
        let req = Request::new(RequestBody::None);
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::POST;
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::DELETE;
        let err = service.call(req).await.err().unwrap();
        assert!(matches!(err, RouteError::First(MethodNotAllowed)));

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::PUT;
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);

        let mut req = Request::new(RequestBody::None);
        *req.method_mut() = Method::TRACE;
        let res = service.call(req).await.ok().unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }

    #[should_panic]
    #[test]
    fn overwrite_method_panic() {
        let _ = Route::new(fn_service(index))
            .methods([Method::POST, Method::PUT])
            .methods([Method::GET, Method::PUT]);
    }

    #[tokio::test]
    async fn route_accept_crate_request() {
        get(fn_service(|_: Request<()>| async {
            Ok::<_, Infallible>(Response::new(()))
        }))
        .build(())
        .await
        .unwrap()
        .call(Request::new(()))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn route_accept_http_request() {
        get(fn_service(|_: http::Request<()>| async {
            Ok::<_, Infallible>(Response::new(()))
        }))
        .build(())
        .await
        .unwrap()
        .call(http::Request::new(()))
        .await
        .unwrap();
    }
}
