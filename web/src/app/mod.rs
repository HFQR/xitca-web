use std::future::Future;

use futures_core::future::LocalBoxFuture;
use xitca_http::{http::Request, RequestBody, ResponseError};
use xitca_http::util::service::{Routable, Router};
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
    = String;
    fn path(&self) -> String {
        String::from("hello")
    }
}

// App keeps a similar API to xitca-web::App. But in real it can be much simpler.

type StateFactory<State> = Box<dyn Fn() -> LocalBoxFuture<'static, State>>;

pub struct App<SF = StateFactory<()>, F = ()> {
    state_factory: SF,
    pub factory: F,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> App {
        Self {
            state_factory: Box::new(|| Box::pin(async {})),
            factory: (),
        }
    }
}

impl App {
    /// Construct App with a thread local state.
    ///
    /// State would still be shared among tasks on the same thread.
    pub fn with_current_thread_state<State>(state: State) -> App<StateFactory<State>>
    where
        State: Clone + 'static,
    {
        Self::with_async_state(Box::new(move || {
            let state = state.clone();
            Box::pin(async move { state })
        }))
    }

    /// Construct App with a thread safe state.
    ///
    /// State would be shared among all tasks and worker threads.
    pub fn with_multi_thread_state<State>(state: State) -> App<StateFactory<State>>
    where
        State: Send + Sync + Clone + 'static,
    {
        Self::with_async_state(Box::new(move || {
            let state = state.clone();
            Box::pin(async move { state })
        }))
    }

    #[doc(hidden)]
    /// Construct App with async closure which it's output would be used as state.
    pub fn with_async_state<SF, Fut>(state_factory: SF) -> App<SF>
    where
        SF: Fn() -> Fut,
        Fut: Future,
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

impl<SF, Fut, F, S, Res, Err, Cfg, IntErr> ServiceFactory<Request<RequestBody>> for App<SF, F>
where
    SF: Fn() -> Fut,
    Fut: Future + 'static,
    F: for<'rb, 'r> ServiceFactory<
        &'rb mut WebRequest<'r, Fut::Output>,
        Service = S,
        Response = Res,
        Error = Err,
        Config = Cfg,
        InitError = IntErr,
    >,
    S: for<'rb, 'r> Service<&'rb mut WebRequest<'r, Fut::Output>, Response = Res, Error = Err> + 'static,
    Err: for<'r> ResponseError<WebRequest<'r, Fut::Output>, Res>,
{
    type Response = Res;
    type Error = Err;
    type Config = Cfg;
    type Service = AppService<Fut::Output, S>;
    type InitError = IntErr;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let state = (&self.state_factory)();
        let service = self.factory.new_service(cfg);
        async {
            let state = state.await;
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

//#[cfg(test)]
mod test {
    use xitca_service::{middleware::Cloneable, ServiceFactory};

    use super::*;

    use crate::response::{ResponseBody, WebResponse};

    struct TestFactory;

    impl ServiceFactory<&'_ mut WebRequest<'_, String>> for TestFactory {
        type Response = WebResponse;
        type Error = ();
        type Config = ();
        type Service = TestService;
        type InitError = ();
        type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

        fn new_service(&self, _: Self::Config) -> Self::Future {
            async { Ok(TestService) }
        }
    }

    struct TestService;

    impl<'r, 's> Service<&'r mut WebRequest<'s, String>> for TestService {
        type Response = WebResponse;
        type Error = ();
        type Ready<'f> = impl Future<Output = Result<(), Self::Error>>;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn ready(&self) -> Self::Ready<'_> {
            async { Ok(()) }
        }

        fn call(&self, req: &'r mut WebRequest<'s, String>) -> Self::Future<'_> {
            async move {
                assert_eq!(req.state(), "state");

                Ok(WebResponse::new(ResponseBody::None))
            }
        }
    }

    //#[tokio::test]
    async fn test_app() {
        let state = String::from("state");
        let app = App::with_current_thread_state(state).service(TestFactory);

        let service = app.new_service(()).await.ok().unwrap();

        let req = Request::default();

        let _ = service.call(req).await.unwrap();
    }

    //#[tokio::test]
    async fn test_handler() {
        use crate::extract::{PathRef, StateRef};
        use crate::service::HandlerService;
        use xitca_http::util::service::{Routable, Router};
        use xitca_service::ServiceFactoryExt;

        async fn handler(StateRef(state): StateRef<'_, String>, PathRef(path): PathRef<'_>) -> String {
            assert_eq!("state", state);
            assert_eq!("/", path);
            state.to_string()
        }

        let state = String::from("state");
        let service = App::with_current_thread_state(state)
            .service(
                Router::new()
                    .insert("/hello", HandlerService::new(handler))
                    .map_err(drop),
            )
            .middleware(Cloneable)
            .new_service(())
            .await
            .ok()
            .unwrap();

        let req = Request::default();

        let _ = service.call(req).await.unwrap();
    }
}
