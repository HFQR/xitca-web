use std::convert::Infallible;
use std::{
    error,
    fmt::{self, Debug, Display, Formatter},
    future::Future,
    marker::PhantomData,
};

use xitca_service::{
    pipeline::PipelineE, ready::ReadyService, BuildService, MapErrorServiceFactory, Service, ServiceFactoryExt,
};

use crate::{
    http::{self, Method},
    request::BorrowReq,
};

macro_rules! method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<R, Req, Res, Err>(
            route: R,
        ) -> Route<
            MapErrorServiceFactory<R, impl Fn(Err) -> RouteError<Err> + Clone>,
            MethodNotAllowedService<Res, Err>,
            1,
        >
        where
            R: BuildService,
            R::Service: Service<Req, Response = Res, Error = Err>,
            Req: BorrowReq<http::Method>,
        {
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
    #[allow(clippy::type_complexity)]
    pub fn new<R, Req, Res, Err>(
        route: R,
    ) -> Route<MapErrorServiceFactory<R, impl Fn(Err) -> RouteError<Err> + Clone>, MethodNotAllowedService<Res, Err>, 0>
    where
        R: BuildService,
        R::Service: Service<Req, Response = Res, Error = Err>,
        Req: BorrowReq<http::Method>,
    {
        Route {
            methods: [],
            route: route.map_err(RouteError::Second),
            next: MethodNotAllowedService::default(),
        }
    }
}

macro_rules! route_method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<N1, Req, Err>(
            self,
            $method_fn: N1,
        ) -> Route<R, Route<MapErrorServiceFactory<N1, impl Fn(Err) -> RouteError<Err> + Clone>, N, 1>, M>
        where
            R: BuildService,
            N1: BuildService,
            N1::Service: Service<Req, Error = Err>,
            Req: BorrowReq<http::Method>,
        {
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

    pub fn next<R1, N1, const M1: usize>(self, next: Route<R1, N1, M1>) -> Route<R, Route<R1, N, M1>, M>
    where
        R: BuildService,
        N1: BuildService,
    {
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

impl<Arg, R, N, const M: usize> BuildService<Arg> for Route<R, N, M>
where
    R: BuildService<Arg>,
    N: BuildService<Arg, Error = R::Error>,
    Arg: Clone,
{
    type Service = Route<R::Service, N::Service, M>;
    type Error = R::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let route = self.route.build(arg.clone());
        let next = self.next.build(arg);

        let methods = self.methods.clone();

        async move {
            let route = route.await?;
            let next = next.await?;
            // re-use Route for Service trait type.
            Ok(Route { methods, route, next })
        }
    }
}

impl<Req, R, N, const M: usize> Service<Req> for Route<R, N, M>
where
    R: Service<Req>,
    N: Service<Req, Response = R::Response, Error = R::Error>,
    Req: BorrowReq<http::Method>,
{
    type Response = R::Response;
    type Error = R::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            if self.methods.contains(req.borrow()) {
                self.route.call(req).await
            } else {
                self.next.call(req).await
            }
        }
    }
}

impl<Req, R, N, const M: usize> ReadyService<Req> for Route<R, N, M>
where
    R: Service<Req>,
    N: Service<Req, Response = R::Response, Error = R::Error>,
    Req: BorrowReq<http::Method>,
{
    type Ready = ();
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async { Ok(()) }
    }
}

/// Error type of Route service.
pub type RouteError<E> = PipelineE<MethodNotAllowed, E>;

/// Error type of Method not allow for route.
pub struct MethodNotAllowed;

impl Debug for MethodNotAllowed {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Method is not allowed")
    }
}

impl Display for MethodNotAllowed {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Method is not allowed")
    }
}

impl error::Error for MethodNotAllowed {}

#[doc(hidden)]
pub struct MethodNotAllowedService<Res, Err>(PhantomData<(Res, Err)>);

impl<Res, Err> Default for MethodNotAllowedService<Res, Err> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<Arg, Res, Err> BuildService<Arg> for MethodNotAllowedService<Res, Err> {
    type Service = Self;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, _: Arg) -> Self::Future {
        async { Ok(MethodNotAllowedService::default()) }
    }
}

impl<Req, Res, Err> Service<Req> for MethodNotAllowedService<Res, Err> {
    type Response = Res;
    type Error = RouteError<Err>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[cold]
    #[inline(never)]
    fn call(&self, _: Req) -> Self::Future<'_> {
        async { Result::Err(RouteError::First(MethodNotAllowed)) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::convert::Infallible;

    use xitca_service::{fn_service, Service};

    use crate::{
        body::{RequestBody, ResponseBody},
        http::{self, Response},
        request::Request,
    };

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
