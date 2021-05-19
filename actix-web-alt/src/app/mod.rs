mod entry;

use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_http_alt::HttpRequest;
use actix_service_alt::{Service, ServiceFactory};

use crate::request::WebRequest;

// App keeps a similar API to actix-web::App. But in real it can be much simpler.

pub struct App<State = (), F = ()> {
    state: State,
    pub factory: F,
}

impl App {
    pub fn new() -> Self {
        Self { state: (), factory: () }
    }
}

impl App {
    /// Construct App with a thread local state.
    ///
    /// State would still be shared among tasks on the same thread.
    pub fn with_current_thread_state<State>(state: State) -> App<State>
    where
        State: Clone + 'static,
    {
        App { state, factory: () }
    }

    /// Construct App with a thread safe state.
    ///
    /// State would be shared among all tasks and worker threads.
    pub fn with_multi_thread_state<State>(state: State) -> App<State>
    where
        State: Send + Sync + Clone + 'static,
    {
        App { state, factory: () }
    }
}

impl<State> App<State>
where
    State: Clone,
{
    pub fn service<F>(self, factory: F) -> App<State, F>
    where
        F: for<'f> ServiceFactory<WebRequest<'f, State>>,
    {
        App {
            state: self.state,
            factory,
        }
    }
}

impl<State, F, S, Res, Err, Cfg, IntErr> ServiceFactory<HttpRequest> for App<State, F>
where
    State: Clone + 'static,
    F: for<'r> ServiceFactory<
        WebRequest<'r, State>,
        Service = S,
        Response = Res,
        Error = Err,
        Config = Cfg,
        InitError = IntErr,
    >,
    S: for<'r> Service<WebRequest<'r, State>, Response = Res, Error = Err> + 'static,
{
    type Response = Res;
    type Error = Err;
    type Config = Cfg;
    type Service = AppService<State, S>;
    type InitError = IntErr;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let state = self.state.clone();
        let service = self.factory.new_service(cfg);
        async {
            let service = service.await?;
            Ok(AppService { service, state })
        }
    }
}

pub struct AppService<State, S> {
    state: State,
    service: S,
}

impl<State, S, Res, Err> Service<HttpRequest> for AppService<State, S>
where
    State: 'static,
    S: for<'s> Service<WebRequest<'s, State>, Response = Res, Error = Err> + 'static,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call<'c>(&'c self, req: HttpRequest) -> Self::Future<'c>
    where
        HttpRequest: 'c,
    {
        async move {
            let req = WebRequest::new(req, &self.state);
            self.service.call(req).await
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    struct TestFactory;

    impl<State: Clone> ServiceFactory<WebRequest<'_, State>> for TestFactory {
        type Response = State;
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

    impl<'r, State> Service<WebRequest<'r, State>> for TestService
    where
        State: Clone,
    {
        type Response = State;
        type Error = ();
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call<'c>(&'c self, req: WebRequest<'r, State>) -> Self::Future<'c>
        where
            'r: 'c,
        {
            async move { Ok(req.state().clone()) }
        }
    }

    #[tokio::test]
    async fn test_app() {
        let state = String::from("state");

        let service = <TestFactory as ServiceFactory<WebRequest<'_, String>>>::new_service(&TestFactory, ())
            .await
            .unwrap();

        let req = WebRequest::with_state(&state);

        service.call(req).await.unwrap();
    }
}
