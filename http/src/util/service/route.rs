use std::{error, fmt, future::Future, marker::PhantomData};

use xitca_service::{Service, ServiceFactory, ServiceFactoryExt};

use crate::http::{Method, Request};

type RouteMethodFactory<Req, R: ServiceFactory<Req>> = impl ServiceFactory<
    Req,
    Response = R::Response,
    Error = RouteError<R::Error>,
    Config = R::Config,
    InitError = R::InitError,
    Service = impl Service<Req, Response = R::Response, Error = RouteError<R::Error>> + Clone,
>;

macro_rules! method {
    ($method_fn: ident, $method: ident) => {
        pub fn $method_fn<R, Req>(
            route: R,
        ) -> Route<RouteMethodFactory<Req, R>, MethodNotAllowed<R::Response, R::Error, R::Config, R::InitError>, 1>
        where
            R: ServiceFactory<Req>,
            R::Service: Clone,
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

impl<R, N, const M: usize> Clone for Route<R, N, M>
where
    R: Clone,
    N: Clone,
{
    fn clone(&self) -> Self {
        Self {
            methods: self.methods.clone(),
            route: self.route.clone(),
            next: self.next.clone(),
        }
    }
}

impl Route<(), (), 0> {
    #[allow(clippy::type_complexity)]
    pub fn new<R, Req>(
        route: R,
    ) -> Route<RouteMethodFactory<Req, R>, MethodNotAllowed<R::Response, R::Error, R::Config, R::InitError>, 0>
    where
        R: ServiceFactory<Req>,
        R::Service: Clone,
    {
        Route {
            methods: [],
            route: route.map_err(RouteError::Service),
            next: MethodNotAllowed::default(),
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
            Route<
                impl ServiceFactory<
                    Req,
                    Response = N1::Response,
                    Error = RouteError<N1::Error>,
                    Config = N1::Config,
                    InitError = N1::InitError,
                    Service = impl Service<Req, Response = N1::Response, Error = RouteError<N1::Error>> + Clone,
                >,
                N,
                1,
            >,
            M,
        >
        where
            R: ServiceFactory<Req>,
            R::Service: Clone,
            N1: ServiceFactory<Req>,
            N1::Service: Clone,
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

    pub fn next<Req, R1, N1, const M1: usize>(self, next: Route<R1, N1, M1>) -> Route<R, Route<R1, N, M1>, M>
    where
        R: ServiceFactory<Req>,
        R::Service: Clone,
        N1: ServiceFactory<Req>,
        N1::Service: Clone,
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

impl<ReqB, R, N, const M: usize> ServiceFactory<Request<ReqB>> for Route<R, N, M>
where
    R: ServiceFactory<Request<ReqB>>,
    N: ServiceFactory<
        Request<ReqB>,
        Response = R::Response,
        Error = R::Error,
        Config = R::Config,
        InitError = R::InitError,
    >,
    R::Config: Clone,
{
    type Response = R::Response;
    type Error = R::Error;
    type Config = R::Config;
    type Service = Route<R::Service, N::Service, M>;
    type InitError = R::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let route = self.route.new_service(cfg.clone());
        let next = self.next.new_service(cfg);

        let methods = self.methods.clone();

        async move {
            let route = route.await?;
            let next = next.await?;
            // re-use Route for Service trait type.
            Ok(Route { methods, route, next })
        }
    }
}

impl<ReqB, R, N, const M: usize> Service<Request<ReqB>> for Route<R, N, M>
where
    R: Service<Request<ReqB>>,
    N: Service<Request<ReqB>, Response = R::Response, Error = R::Error>,
{
    type Response = R::Response;
    type Error = R::Error;
    type Ready<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        async { Ok(()) }
    }

    #[inline]
    fn call(&self, req: Request<ReqB>) -> Self::Future<'_> {
        async move {
            if self.methods.contains(req.method()) {
                self.route.ready().await?;
                self.route.call(req).await
            } else {
                self.next.call(req).await
            }
        }
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
pub struct MethodNotAllowed<Res, Err, Cfg, InitErr>(PhantomData<(Res, Err, Cfg, InitErr)>);

impl<Res, Err, Cfg, InitErr> Default for MethodNotAllowed<Res, Err, Cfg, InitErr> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<Res, Err, Cfg, InitErr> Clone for MethodNotAllowed<Res, Err, Cfg, InitErr> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<Req, Res, Err, Cfg, InitErr> ServiceFactory<Req> for MethodNotAllowed<Res, Err, Cfg, InitErr> {
    type Response = Res;
    type Error = RouteError<Err>;
    type Config = Cfg;
    type Service = Self;
    type InitError = InitErr;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let this = self.clone();
        async { Ok(this) }
    }
}

impl<Req, Res, Err, Cfg, InitErr> Service<Req> for MethodNotAllowed<Res, Err, Cfg, InitErr> {
    type Response = Res;
    type Error = RouteError<Err>;
    type Ready<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        async { Ok(()) }
    }

    #[inline]
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
        http::Response,
    };

    async fn index(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Infallible> {
        Ok(Response::new(ResponseBody::None))
    }

    #[tokio::test]
    async fn route_fn() {
        let route = get(fn_service(index))
            .post(fn_service(index))
            .trace(fn_service(index))
            .transform_fn(|s, req| async move { s.call(req).await });

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
}
