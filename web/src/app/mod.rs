use std::{
    convert::Infallible,
    future::{ready, Future, Ready},
    marker::PhantomData,
};

use xitca_http::{Request, RequestBody};
use xitca_service::{
    ready::ReadyService, AsyncClosure, EnclosedFactory, EnclosedFnFactory, Service, ServiceFactory, ServiceFactoryExt,
};

use crate::request::WebRequest;

// App keeps a similar API to xitca-web::App. But in real it can be much simpler.

pub struct App<SF = (), S = (), F = ()> {
    state_factory: SF,
    factory: F,
    _state: PhantomData<S>,
}

impl App {
    pub fn new() -> App<impl Fn() -> Ready<Result<(), ()>>> {
        App {
            state_factory: || ready(Ok(())),
            factory: (),
            _state: PhantomData,
        }
    }
}

impl App {
    /// Construct App with a thread local state.
    ///
    /// State would still be shared among tasks on the same thread.
    pub fn with_current_thread_state<State>(state: State) -> App<impl Fn() -> Ready<Result<State, Infallible>>, State>
    where
        State: Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    /// Construct App with a thread safe state.
    ///
    /// State would be shared among all tasks and worker threads.
    pub fn with_multi_thread_state<State>(state: State) -> App<impl Fn() -> Ready<Result<State, Infallible>>, State>
    where
        State: Send + Sync + Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    #[doc(hidden)]
    /// Construct App with async closure which it's output would be used as state.
    pub fn with_async_state<SF, Fut, T, E>(state_factory: SF) -> App<SF, T>
    where
        SF: Fn() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        App {
            state_factory,
            factory: (),
            _state: PhantomData,
        }
    }
}

impl<SF, S, F> App<SF, S, F> {
    pub fn service<F1>(self, factory: F1) -> App<SF, S, F1>
    where
        App<SF, S, F1>: ServiceFactory<Request<RequestBody>, ()>,
    {
        App {
            state_factory: self.state_factory,
            factory,
            _state: PhantomData,
        }
    }

    pub fn enclosed<Req, T>(self, transform: T) -> App<SF, S, EnclosedFactory<F, T>>
    where
        F: ServiceFactory<Req>,
        T: ServiceFactory<Req, F::Service> + Clone,
    {
        App {
            state_factory: self.state_factory,
            factory: self.factory.enclosed(transform),
            _state: PhantomData,
        }
    }

    pub fn enclosed_fn<Req, T>(self, transform: T) -> App<SF, S, EnclosedFnFactory<F, T>>
    where
        F: ServiceFactory<Req>,
        T: for<'s> AsyncClosure<(&'s F::Service, Req)> + Clone,
    {
        App {
            state_factory: self.state_factory,
            factory: self.factory.enclosed_fn(transform),
            _state: PhantomData,
        }
    }
}

impl<SF, Fut, State, StateErr, F, S, Arg, Res, Err> ServiceFactory<Request<RequestBody>, Arg> for App<SF, State, F>
where
    SF: Fn() -> Fut,
    Fut: Future<Output = Result<State, StateErr>> + 'static,
    State: 'static,
    F: for<'rb, 'r> ServiceFactory<&'rb mut WebRequest<'r, State>, Arg, Service = S, Response = Res, Error = Err>,
    S: for<'rb, 'r> Service<&'rb mut WebRequest<'r, State>, Response = Res, Error = Err> + 'static,
    Err: From<StateErr>,
{
    type Response = Res;
    type Error = Err;
    type Service = AppService<State, S>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let state = (self.state_factory)();
        let service = self.factory.new_service(arg);
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
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<RequestBody>) -> Self::Future<'_> {
        async move {
            let mut req = WebRequest::new(req, &self.state);
            self.service.call(&mut req).await
        }
    }
}

impl<State, S, R, Res, Err> ReadyService<Request<RequestBody>> for AppService<State, S>
where
    State: 'static,
    S: for<'r, 's> ReadyService<&'r mut WebRequest<'s, State>, Response = Res, Error = Err, Ready = R> + 'static,
{
    type Ready = R;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>>;

    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move { self.service.ready().await }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use xitca_http::util::service::GenericRouter;

    use crate::{
        extract::{PathRef, StateRef},
        http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE},
        object::WebObjectConstructor,
        service::HandlerService,
    };

    async fn handler(
        StateRef(state): StateRef<'_, String>,
        PathRef(path): PathRef<'_>,
        req: &WebRequest<'_, String>,
    ) -> String {
        assert_eq!("state", state);
        assert_eq!(state, req.state());
        assert_eq!("/", path);
        assert_eq!(path, req.req().uri().path());
        state.to_string()
    }

    #[derive(Clone)]
    struct Middleware;

    impl<S, State, Res, Err> ServiceFactory<&mut WebRequest<'_, State>, S> for Middleware
    where
        S: for<'r, 's> Service<&'r mut WebRequest<'s, State>, Response = Res, Error = Err>,
    {
        type Response = Res;
        type Error = Err;
        type Service = MiddlewareService<S>;
        type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

        fn new_service(&self, service: S) -> Self::Future {
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
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

        fn call(&self, req: &'r mut WebRequest<'s, State>) -> Self::Future<'_> {
            async move { self.0.call(req).await }
        }
    }

    #[tokio::test]
    async fn test_app() {
        async fn middleware_fn<S, State, Res, Err>(service: &S, req: &mut WebRequest<'_, State>) -> Result<Res, Err>
        where
            S: for<'r, 's> Service<&'r mut WebRequest<'s, State>, Response = Res, Error = Err>,
        {
            service.call(req).await
        }

        let state = String::from("state");

        let service = App::with_current_thread_state(state)
            .service(
                GenericRouter::with_custom_object::<WebObjectConstructor<_>>()
                    .insert("/", HandlerService::new(handler).enclosed(Middleware))
                    .map_err(|e| -> Infallible { panic!("error {}", e) }),
            )
            .enclosed(Middleware)
            .enclosed_fn(middleware_fn)
            .new_service(())
            .await
            .ok()
            .unwrap();

        let req = Request::default();

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status().as_u16(), 200);
        assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), TEXT_UTF8);
    }
}
