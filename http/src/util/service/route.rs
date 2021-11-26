use std::{
    future::Future,
    future::{ready, Ready},
    marker::PhantomData,
};

use xitca_service::{Service, ServiceFactory, ServiceFactoryExt};

use crate::http::{Method, Request};

macro_rules! method {
    ($method: ident; $($req: ident), *) => {
        pub fn $method<F, Req>(
            factory: F,
        ) -> Route<
            F::Response,
            F::Error,
            F::Config,
            F::InitError,
            // This is a hack to generate opaque return type repeatedly.
            $(
                impl ServiceFactory<
                    $req,
                    Response = F::Response,
                    Error = RouteError<F::Error>,
                    Config = F::Config,
                    InitError = F::InitError,
                >,
            ) *
        >
        where
            F: ServiceFactory<Req>,
        {
            Route::new().$method(factory)
        }
    };
}

method!(get; Req, Req, Req);
method!(post; Req, Req, Req);
method!(put; Req, Req, Req);

macro_rules! route {
    ($($method: ident), *) => {
        #[allow(non_camel_case_types)]
        pub struct Route<
            Res,
            Err,
            Cfg,
            InitErr,
            $($method = MethodNotAllowed<Res, Err, Cfg, InitErr>), *
        > {
            $($method: $method), *,
            _phantom: PhantomData<(Res, Err, Cfg, InitErr)>,
        }
    };
}

route!(get, post, put);

impl<Res, Err, Cfg, InitErr> Default for Route<Res, Err, Cfg, InitErr> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Res, Err, Cfg, InitErr> Route<Res, Err, Cfg, InitErr> {
    pub fn new() -> Self {
        Self {
            get: Default::default(),
            post: Default::default(),
            put: Default::default(),
            _phantom: PhantomData,
        }
    }
}

impl<Res, Err, Cfg, InitErr, GET, POST, PUT> Route<Res, Err, Cfg, InitErr, GET, POST, PUT> {
    pub fn get<GET1, Req>(
        self,
        factory: GET1,
    ) -> Route<
        Res,
        Err,
        Cfg,
        InitErr,
        impl ServiceFactory<Req, Response = Res, Error = RouteError<Err>, Config = Cfg, InitError = InitErr>,
        POST,
        PUT,
    >
    where
        GET1: ServiceFactory<Req, Response = Res, Error = Err, Config = Cfg, InitError = InitErr>,
    {
        Route {
            get: factory.map_err(RouteError::Service),
            post: self.post,
            put: self.put,
            _phantom: PhantomData,
        }
    }

    pub fn post<POST1, Req>(
        self,
        factory: POST1,
    ) -> Route<
        Res,
        Err,
        Cfg,
        InitErr,
        GET,
        impl ServiceFactory<Req, Response = Res, Error = RouteError<Err>, Config = Cfg, InitError = InitErr>,
        PUT,
    >
    where
        POST1: ServiceFactory<Req, Response = Res, Error = Err, Config = Cfg, InitError = InitErr>,
    {
        Route {
            get: self.get,
            post: factory.map_err(RouteError::Service),
            put: self.put,
            _phantom: PhantomData,
        }
    }

    pub fn put<PUT1, Req>(
        self,
        factory: PUT1,
    ) -> Route<
        Res,
        Err,
        Cfg,
        InitErr,
        GET,
        POST,
        impl ServiceFactory<Req, Response = Res, Error = RouteError<Err>, Config = Cfg, InitError = InitErr>,
    >
    where
        PUT1: ServiceFactory<Req, Response = Res, Error = Err, Config = Cfg, InitError = InitErr>,
    {
        Route {
            get: self.get,
            post: self.post,
            put: factory.map_err(RouteError::Service),
            _phantom: PhantomData,
        }
    }
}

impl<ReqB, Res, Err, Cfg, InitErr, GET, POST, PUT> ServiceFactory<Request<ReqB>>
    for Route<Res, Err, Cfg, InitErr, GET, POST, PUT>
where
    GET: ServiceFactory<Request<ReqB>, Response = Res, Error = RouteError<Err>, Config = Cfg, InitError = InitErr>,
    POST: ServiceFactory<Request<ReqB>, Response = Res, Error = RouteError<Err>, Config = Cfg, InitError = InitErr>,
    PUT: ServiceFactory<Request<ReqB>, Response = Res, Error = RouteError<Err>, Config = Cfg, InitError = InitErr>,
    Cfg: Clone,
{
    type Response = Res;
    type Error = RouteError<Err>;
    type Config = Cfg;
    type Service = RouteService<GET::Service, POST::Service, PUT::Service>;
    type InitError = InitErr;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let get = self.get.new_service(cfg.clone());
        let post = self.post.new_service(cfg.clone());
        let put = self.put.new_service(cfg);

        async move {
            let get = get.await?;
            let post = post.await?;
            let put = put.await?;

            Ok(RouteService { get, post, put })
        }
    }
}

macro_rules! route_service {
    ($($method: ident), *) => {
        #[allow(non_camel_case_types)]
        pub struct RouteService<$($method), *> {
            $($method: $method), *
        }
    }
}

route_service!(get, post, put);

impl<ReqB, GET, POST, PUT> Service<Request<ReqB>> for RouteService<GET, POST, PUT>
where
    GET: Service<Request<ReqB>>,
    POST: Service<Request<ReqB>, Response = GET::Response, Error = GET::Error>,
    PUT: Service<Request<ReqB>, Response = GET::Response, Error = GET::Error>,
{
    type Response = GET::Response;
    type Error = GET::Error;
    type Ready<'f>
    where
        Self: 'f,
    = Ready<Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        ready(Ok(()))
    }

    #[inline]
    fn call(&self, req: Request<ReqB>) -> Self::Future<'_> {
        async move {
            match *req.method() {
                Method::GET => self.get.call(req).await,
                Method::POST => self.post.call(req).await,
                Method::PUT => self.put.call(req).await,
                _ => unimplemented!("{:?} Method can not be handled", req.method()),
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
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        ready(Ok(self.clone()))
    }
}

impl<Req, Res, Err, Cfg, InitErr> Service<Req> for MethodNotAllowed<Res, Err, Cfg, InitErr> {
    type Response = Res;
    type Error = RouteError<Err>;
    type Ready<'f>
    where
        Self: 'f,
    = Ready<Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = Ready<Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        ready(Ok(()))
    }

    #[inline]
    fn call(&self, _: Req) -> Self::Future<'_> {
        ready(Err(RouteError::MethodNotAllowed))
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
    async fn route() {
        let route = get(fn_service(index)).post(fn_service(index));

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
}
