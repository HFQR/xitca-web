mod router;

use std::future::{ready, Future, Ready};

use xitca_http::util::service::Routable;
use xitca_http::{http::Request, RequestBody, ResponseError};
use xitca_service::{
    Request as ReqTrait, RequestSpecs, Service, ServiceFactory, ServiceFactoryExt, Transform, TransformFactory,
};

use crate::request::WebRequest;

impl<'a, 'b, S: 'static> ReqTrait<'a, &'a &'b ()> for &'static mut WebRequest<'static, S> {
    type Type = &'a mut WebRequest<'b, S>;
}

impl<'a, 'b, S: 'static> RequestSpecs<&'a mut WebRequest<'b, S>> for &'static mut WebRequest<'static, S> {
    type Lifetime = &'a ();
    type Lifetimes = &'a &'b ();
}

impl<S: 'static> Routable for &mut WebRequest<'_, S> {
    type Path<'a>
    where
        Self: 'a,
    = &'a str;

    fn path(&self) -> &str {
        // TODO consider url decoding and Cow
        self.req().uri().path()
    }
}

// App keeps a similar API to xitca-web::App. But in real it can be much simpler.

pub struct App<SF = (), F = ()> {
    state_factory: SF,
    factory: F,
}

impl App {
    pub fn new() -> App<impl Fn() -> Ready<Result<(), ()>>> {
        App {
            state_factory: || ready(Ok(())),
            factory: (),
        }
    }
}

impl App {
    /// Construct App with a thread local state.
    ///
    /// State would still be shared among tasks on the same thread.
    pub fn with_current_thread_state<State>(state: State) -> App<impl Fn() -> Ready<Result<State, ()>>>
    where
        State: Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    /// Construct App with a thread safe state.
    ///
    /// State would be shared among all tasks and worker threads.
    pub fn with_multi_thread_state<State>(state: State) -> App<impl Fn() -> Ready<Result<State, ()>>>
    where
        State: Send + Sync + Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    #[doc(hidden)]
    /// Construct App with async closure which it's output would be used as state.
    pub fn with_async_state<SF, Fut, T, E>(state_factory: SF) -> App<SF>
    where
        SF: Fn() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        App {
            state_factory,
            factory: (),
        }
    }
}

impl<SF, F> App<SF, F> {
    pub fn service<F1>(self, factory: F1) -> App<SF, F1> {
        App {
            state_factory: self.state_factory,
            factory,
        }
    }

    pub fn middleware<Req, T>(self, transform: T) -> App<SF, TransformFactory<F, T>>
    where
        F: ServiceFactory<Req>,
        T: Transform<F::Service, Req>,
        F::InitError: From<T::InitError>,
    {
        App {
            state_factory: self.state_factory,
            factory: self.factory.transform(transform),
        }
    }
}

impl<SF, Fut, State, StateErr, F, S, Res, Err, Cfg, IntErr> ServiceFactory<Request<RequestBody>> for App<SF, F>
where
    SF: Fn() -> Fut,
    Fut: Future<Output = Result<State, StateErr>> + 'static,
    State: 'static,
    StateErr: 'static,
    IntErr: From<StateErr>,
    F: for<'rb, 'r> ServiceFactory<
        &'rb mut WebRequest<'r, State>,
        Service = S,
        Response = Res,
        Error = Err,
        Config = Cfg,
        InitError = IntErr,
    >,
    S: for<'rb, 'r> Service<&'rb mut WebRequest<'r, State>, Response = Res, Error = Err> + 'static,
    Err: for<'r> ResponseError<WebRequest<'r, State>, Res>,
{
    type Response = Res;
    type Error = Err;
    type Config = Cfg;
    type Service = AppService<State, S>;
    type InitError = IntErr;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let state = (&self.state_factory)();
        let service = self.factory.new_service(cfg);
        async {
            let state = state.await?;
            let service = service.await?;
            Ok(AppService { service, state })
        }
    }
}

pub struct AppService<State, S> {
    state: State,
    service: S,
}

impl<State, S, Res, Err> Service<Request<RequestBody>> for AppService<State, S>
where
    State: 'static,
    S: for<'r, 's> Service<&'r mut WebRequest<'s, State>, Response = Res, Error = Err> + 'static,
    Err: for<'r> ResponseError<WebRequest<'r, State>, Res>,
{
    type Response = Res;
    type Error = Err;
    type Ready<'f> = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self.service.ready()
    }

    fn call(&self, req: Request<RequestBody>) -> Self::Future<'_> {
        async move {
            let mut req = WebRequest::new(req, &self.state);
            let res = self
                .service
                .call(&mut req)
                .await
                .unwrap_or_else(|e| ResponseError::response_error(e, &mut req));

            Ok(res)
        }
    }
}

#[cfg(test)]
mod test {
    use xitca_http::util::service::Router;

    use super::*;

    use crate::{
        extract::{PathRef, StateRef},
        http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE},
        service::HandlerService,
    };

    async fn handler(
        StateRef(state): StateRef<'_, String>,
        PathRef(path): PathRef<'_>,
        req: &WebRequest<'_, String>,
    ) -> String {
        assert_eq!("state", state);
        assert_eq!(state, req.state());
        assert_eq!("/test", path);
        assert_eq!(path, req.req().uri().path());
        state.to_string()
    }

    #[derive(Clone)]
    struct Middleware;

    impl<S, State, Res, Err> Transform<S, &mut WebRequest<'_, State>> for Middleware
    where
        S: for<'r, 's> Service<&'r mut WebRequest<'s, State>, Response = Res, Error = Err>,
    {
        type Response = Res;
        type Error = Err;
        type Transform = MiddlewareService<S>;
        type InitError = ();
        type Future = impl Future<Output = Result<Self::Transform, Self::InitError>>;

        fn new_transform(&self, service: S) -> Self::Future {
            async move { Ok(MiddlewareService(service)) }
        }
    }

    struct MiddlewareService<S>(S);

    impl<'r, 's, S, State, Res, Err> Service<&'r mut WebRequest<'s, State>> for MiddlewareService<S>
    where
        S: for<'r1, 's1> Service<&'r1 mut WebRequest<'s1, State>, Response = Res, Error = Err>,
    {
        type Response = Res;
        type Error = Err;
        type Ready<'f>
        where
            Self: 'f,
        = impl Future<Output = Result<(), Self::Error>>;
        type Future<'f>
        where
            Self: 'f,
        = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn ready(&self) -> Self::Ready<'_> {
            async move { self.0.ready().await }
        }

        fn call(&self, req: &'r mut WebRequest<'s, State>) -> Self::Future<'_> {
            async move { self.0.call(req).await }
        }
    }

    #[tokio::test]
    async fn test_app() {
        let state = String::from("state");

        let service = App::with_current_thread_state(state)
            .service(
                Router::new()
                    .insert("/test", HandlerService::new(handler))
                    .map_err(drop),
            )
            .middleware(Middleware)
            .new_service(())
            .await
            .ok()
            .unwrap();

        let req = Request::get("/test").body(RequestBody::default()).unwrap();

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status().as_u16(), 200);
        assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), TEXT_UTF8);
    }
}
