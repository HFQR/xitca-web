use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_http_alt::HttpRequest;
use actix_service_alt::{Service, ServiceFactory};

use crate::request::WebRequest;
use std::marker::PhantomData;

// App keeps a similar API to actix-web::App. But in real it can be much simpler.

pub struct App<F = (), D = ()>
where
    D: Send + Sync + Clone,
{
    factory: F,
    data: D,
}

impl App {
    pub fn new() -> Self {
        Self {
            factory: (),
            data: (),
        }
    }
}

impl<D> App<(), D>
where
    D: Send + Sync + Clone,
{
    pub fn with_state(data: D) -> Self {
        Self { factory: (), data }
    }
}

impl<F, D> App<F, D>
where
    D: Send + Sync + Clone,
{
    pub fn service<F2>(self, factory: F2) -> App<F2, D> {
        App {
            factory,
            data: self.data,
        }
    }
}

#[rustfmt::skip]
impl<F, D, S> ServiceFactory<HttpRequest> for App<F, D>
where
    D: Send + Sync + Clone + 'static,
    F: for<'r> ServiceFactory<WebRequest<'r, D>, Service = S, Config = (), InitError = ()>,
    S: for<'r> Service<Request<'r> = WebRequest<'r, D>> + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Config = ();
    type Service = AppService<S, D>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let data = self.data.clone();
        let service = self.factory.new_service(());
        async {
            let service = service.await?;
            Ok(AppService {
                service,
                data
            })
        }
    }
}

pub struct AppService<S, D> {
    service: S,
    data: D,
}

#[rustfmt::skip]
impl<S, D> Service for AppService<S, D>
where
    S: for<'r> Service<Request<'r> = WebRequest<'r, D>> + 'static,
    D: 'static
{
    type Request<'r> = HttpRequest;
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
        async move {
            let req = WebRequest {
                http: &req,
                data: &self.data
            };

            self.service.call(req).await
        }
    }
}
