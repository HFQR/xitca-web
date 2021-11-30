#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use std::{
    error, fmt,
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
            Req,
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
                    Service = impl Service<
                        $req,
                        Response = F::Response,
                        Error = RouteError<F::Error>,
                    > + Clone
                >,
            ) *
        >
        where
            F: ServiceFactory<Req>,
            F::Service: Clone
        {
            Route::new().$method(factory)
        }
    };
}

macro_rules! route {
    ($($method: ident), *; $($req: ident), *) => {
        pub struct Route<
            Req,
            Res,
            Err,
            Cfg,
            InitErr,
            $(
                $method = MethodNotAllowed<Res, Err, Cfg, InitErr>,
            )*
        > {
            $(
                $method: $method,
            )*
            _phantom: PhantomData<(Req, Res, Err, Cfg, InitErr)>,
        }

        impl<Req, Res, Err, Cfg, InitErr> Route<Req, Res, Err, Cfg, InitErr> {
            pub fn new() -> Self {
                Self {
                    $(
                        $method: Default::default(),
                    )*
                    _phantom: PhantomData,
                }
            }
        }

        impl<Req, Res, Err, Cfg, InitErr> Default for Route<Req, Res, Err, Cfg, InitErr> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<ReqB, Res, Err, Cfg, InitErr, $($method), *> ServiceFactory<Request<ReqB>>
            for Route<Request<ReqB>, Res, Err, Cfg, InitErr, $($method), *>
        where
            $(
                $method: ServiceFactory<Request<ReqB>, Response = Res, Error = RouteError<Err>, Config = Cfg, InitError = InitErr>,
            )*
            Cfg: Clone,
        {
            type Response = Res;
            type Error = RouteError<Err>;
            type Config = Cfg;
            type Service = RouteService<$($method::Service), *>;
            type InitError = InitErr;
            type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

            fn new_service(&self, cfg: Self::Config) -> Self::Future {
                let ($($method), *) = ($(self.$method.new_service(cfg.clone())), *);

                async move {
                    let ($($method), *) = ($($method.await?), *);
                    Ok(RouteService { $($method), * })
                }
            }
        }

        impl<Req, Res, Err, Cfg, InitErr, $($method), *> Route<Req, Res, Err, Cfg, InitErr, $($method), *>
        where
            $(
                $method: ServiceFactory<Req, Response = Res, Error = RouteError<Err>, Config = Cfg, InitError = InitErr>,
                $method::Service: Clone,
            )*
        {

            route!(get, GET; POST, PUT, DELETE, HEAD, OPTIONS, CONNECT, PATCH, TRACE; $($req),*);

            route!(post, POST; GET, PUT, DELETE, HEAD, OPTIONS, CONNECT, PATCH, TRACE; $($req),*);

            route!(put, PUT; GET, POST, DELETE, HEAD, OPTIONS, CONNECT, PATCH, TRACE; $($req),*);

            route!(delete, DELETE; GET, POST, PUT, HEAD, OPTIONS, CONNECT, PATCH, TRACE; $($req),*);

            route!(head, HEAD; GET, POST, PUT, DELETE, OPTIONS, CONNECT, PATCH, TRACE; $($req),*);

            route!(options, OPTIONS; GET, POST, PUT, DELETE, HEAD, CONNECT, PATCH, TRACE; $($req),*);

            route!(connect, CONNECT; GET, POST, PUT, DELETE, HEAD, OPTIONS, PATCH, TRACE; $($req),*);

            route!(patch, PATCH; GET, POST, PUT, DELETE, HEAD, OPTIONS, CONNECT, TRACE; $($req),*);

            route!(trace, TRACE; GET, POST, PUT, DELETE, HEAD, OPTIONS, CONNECT, PATCH; $($req),*);
        }
    };
    // Generate Route::method
    (
        $method: ident, $method_ty: ident;
        $($untouched_method: ident),*;
        $($req: ident),*
    ) => {
        pub fn $method<F1>(self, factory: F1) -> Route<
            Req,
            Res,
            Err,
            Cfg,
            InitErr,
            $(
                impl ServiceFactory<
                    $req,
                    Response = Res,
                    Error = RouteError<Err>,
                    Config = Cfg,
                    InitError = InitErr,
                    Service = impl Service<
                        $req,
                        Response = Res,
                        Error = RouteError<Err>
                    > + Clone
                >,
            )*
        >
        where
            F1: ServiceFactory<Req, Response = Res, Error = Err, Config = Cfg, InitError = InitErr>,
            F1::Service: Clone
        {
            Route {
                $method_ty: factory.map_err(RouteError::Service),
                $(
                    $untouched_method: self.$untouched_method,
                )*
                _phantom: PhantomData
            }
        }
    }
}

macro_rules! route_service {
    ($($method: ident), *) => {
        #[allow(non_camel_case_types)]
        pub struct RouteService<$($method), *> {
            $($method: $method), *
        }

        impl<$($method: Clone), *> Clone for RouteService<$($method), *> {
            fn clone(&self) -> Self {
                Self {
                    $(
                        $method: self.$method.clone()
                    ), *
                }
            }
        }

        impl<ReqB, Res, Err, $($method), *> Service<Request<ReqB>> for RouteService<$($method), *>
        where
             $(
                $method: Service<Request<ReqB>, Response = Res, Error = RouteError<Err>>
             ), *
        {
            type Response = Res;
            type Error = RouteError<Err>;
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
                        $(
                            Method::$method => self.$method.call(req).await,
                        ) *
                        _ => Err(RouteError::MethodNotAllowed),
                    }
                }
            }
        }
    }
}

method!(get; Req, Req, Req, Req, Req, Req, Req, Req, Req);
method!(post; Req, Req, Req, Req, Req, Req, Req, Req, Req);
method!(put; Req, Req, Req, Req, Req, Req, Req, Req, Req);
method!(delete; Req, Req, Req, Req, Req, Req, Req, Req, Req);
method!(head; Req, Req, Req, Req, Req, Req, Req, Req, Req);
method!(options; Req, Req, Req, Req, Req, Req, Req, Req, Req);
method!(connect; Req, Req, Req, Req, Req, Req, Req, Req, Req);
method!(patch; Req, Req, Req, Req, Req, Req, Req, Req, Req);
method!(trace; Req, Req, Req, Req, Req, Req, Req, Req, Req);

route!(GET, POST, PUT, DELETE, HEAD, OPTIONS, CONNECT, PATCH, TRACE; Req, Req, Req, Req, Req, Req, Req, Req, Req);

route_service!(GET, POST, PUT, DELETE, HEAD, OPTIONS, CONNECT, PATCH, TRACE);

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
