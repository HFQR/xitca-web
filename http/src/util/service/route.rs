use std::{borrow::Borrow, error, fmt, future::Future, marker::PhantomData};

use xitca_service::{ready::ReadyService, MapErrorServiceFactory, Service, ServiceFactory, ServiceFactoryExt};

use crate::http::{self, Method};

macro_rules! method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<R, Req, ReqB>(
            route: R,
        ) -> Route<
            MapErrorServiceFactory<R, impl Fn(R::Error) -> RouteError<R::Error> + Clone>,
            MethodNotAllowed<R::Response, R::Error>,
            ReqB,
            1,
        >
        where
            R: ServiceFactory<Req>,
            Req: Borrow<http::Request<ReqB>>,
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

pub struct Route<R, N, ReqB, const M: usize> {
    methods: [Method; M],
    route: R,
    next: N,
    _req_body: PhantomData<ReqB>,
}

impl<R, N, ReqB, const M: usize> Clone for Route<R, N, ReqB, M>
where
    R: Clone,
    N: Clone,
{
    fn clone(&self) -> Self {
        Self {
            methods: self.methods.clone(),
            route: self.route.clone(),
            next: self.next.clone(),
            _req_body: PhantomData,
        }
    }
}

impl<ReqB> Route<(), (), ReqB, 0> {
    #[allow(clippy::type_complexity)]
    pub fn new<R, Req>(
        route: R,
    ) -> Route<
        MapErrorServiceFactory<R, impl Fn(R::Error) -> RouteError<R::Error> + Clone>,
        MethodNotAllowed<R::Response, R::Error>,
        ReqB,
        0,
    >
    where
        R: ServiceFactory<Req>,
        Req: Borrow<http::Request<ReqB>>,
    {
        Route {
            methods: [],
            route: route.map_err(RouteError::Service),
            next: MethodNotAllowed::default(),
            _req_body: PhantomData,
        }
    }
}

macro_rules! route_method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<N1, Req>(
            self,
            $method_fn: N1,
        ) -> Route<
            R,
            Route<MapErrorServiceFactory<N1, impl Fn(N1::Error) -> RouteError<N1::Error> + Clone>, N, ReqB, 1>,
            ReqB,
            M,
        >
        where
            R: ServiceFactory<Req>,
            N1: ServiceFactory<Req>,
            Req: Borrow<http::Request<ReqB>>,
        {
            self.next(Route::new($method_fn).methods([Method::$method]))
        }
    };
}

impl<R, N, ReqB, const M: usize> Route<R, N, ReqB, M> {
    pub fn methods<const M1: usize>(self, methods: [Method; M1]) -> Route<R, N, ReqB, M1> {
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
            _req_body: PhantomData,
        }
    }

    pub fn next<Req, R1, N1, const M1: usize>(
        self,
        next: Route<R1, N1, ReqB, M1>,
    ) -> Route<R, Route<R1, N, ReqB, M1>, ReqB, M>
    where
        R: ServiceFactory<Req>,
        N1: ServiceFactory<Req>,
        Req: Borrow<http::Request<ReqB>>,
    {
        Route {
            methods: self.methods,
            route: self.route,
            next: Route {
                methods: next.methods,
                route: next.route,
                next: self.next,
                _req_body: PhantomData,
            },
            _req_body: PhantomData,
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

impl<Req, ReqB, Arg, R, N, const M: usize> ServiceFactory<Req, Arg> for Route<R, N, ReqB, M>
where
    R: ServiceFactory<Req, Arg>,
    N: ServiceFactory<Req, Arg, Response = R::Response, Error = R::Error>,
    Req: Borrow<http::Request<ReqB>>,
    Arg: Clone,
{
    type Response = R::Response;
    type Error = R::Error;
    type Service = Route<R::Service, N::Service, ReqB, M>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let route = self.route.new_service(arg.clone());
        let next = self.next.new_service(arg);

        let methods = self.methods.clone();

        async move {
            let route = route.await?;
            let next = next.await?;
            // re-use Route for Service trait type.
            Ok(Route {
                methods,
                route,
                next,
                _req_body: PhantomData,
            })
        }
    }
}

impl<Req, ReqB, R, N, const M: usize> Service<Req> for Route<R, N, ReqB, M>
where
    R: Service<Req>,
    N: Service<Req, Response = R::Response, Error = R::Error>,
    Req: Borrow<http::Request<ReqB>>,
{
    type Response = R::Response;
    type Error = R::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            if self.methods.contains(req.borrow().method()) {
                self.route.call(req).await
            } else {
                self.next.call(req).await
            }
        }
    }
}

impl<Req, ReqB, R, N, const M: usize> ReadyService<Req> for Route<R, N, ReqB, M>
where
    R: Service<Req>,
    N: Service<Req, Response = R::Response, Error = R::Error>,
    Req: Borrow<http::Request<ReqB>>,
{
    type Ready = ();
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async { Ok(()) }
    }
}

/// Error type of Route service.
pub enum RouteError<E> {
    /// Method is not allowed for route service.
    MethodNotAllowed,
    /// Error type of the inner service.
    Service(E),
}

impl<E: fmt::Debug> fmt::Debug for RouteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MethodNotAllowed => write!(f, "MethodNotAllowed"),
            Self::Service(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl<E: fmt::Display> fmt::Display for RouteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MethodNotAllowed => write!(f, "MethodNotAllowed"),
            Self::Service(ref e) => write!(f, "{}", e),
        }
    }
}

impl<E> error::Error for RouteError<E> where E: fmt::Debug + fmt::Display {}

#[doc(hidden)]
pub struct MethodNotAllowed<Res, Err>(PhantomData<(Res, Err)>);

impl<Res, Err> Default for MethodNotAllowed<Res, Err> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<Res, Err> Clone for MethodNotAllowed<Res, Err> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<Req, Arg, Res, Err> ServiceFactory<Req, Arg> for MethodNotAllowed<Res, Err> {
    type Response = Res;
    type Error = RouteError<Err>;
    type Service = Self;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, _: Arg) -> Self::Future {
        let this = self.clone();
        async { Ok(this) }
    }
}

impl<Req, Res, Err> Service<Req> for MethodNotAllowed<Res, Err> {
    type Response = Res;
    type Error = RouteError<Err>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[cold]
    #[inline(never)]
    fn call(&self, _: Req) -> Self::Future<'_> {
        async { Err(RouteError::MethodNotAllowed) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::convert::Infallible;

    use xitca_service::fn_service;

    use crate::{
        body::{RequestBody, ResponseBody},
        http::{self, Response},
        request::Request,
    };

    async fn index(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Infallible> {
        Ok(Response::new(ResponseBody::None))
    }

    #[tokio::test]
    async fn route_fn() {
        let route = get(fn_service(index))
            .post(fn_service(index))
            .trace(fn_service(index))
            .enclosed_fn(|s, req| async move { s.call(req).await });

        let service = route.new_service(()).await.ok().unwrap();
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
        assert!(matches!(err, RouteError::MethodNotAllowed));
    }

    #[tokio::test]
    async fn route_mixed() {
        let route = get(fn_service(index)).next(Route::new(fn_service(index)).methods([Method::POST, Method::PUT]));

        let service = route.new_service(()).await.ok().unwrap();
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
        assert!(matches!(err, RouteError::MethodNotAllowed));

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

        let service = route.new_service(()).await.ok().unwrap();
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
        assert!(matches!(err, RouteError::MethodNotAllowed));

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
        .new_service(())
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
        .new_service(())
        .await
        .unwrap()
        .call(http::Request::new(()))
        .await
        .unwrap();
    }
}
