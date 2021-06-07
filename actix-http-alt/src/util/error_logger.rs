use std::fmt::Debug;

use actix_service_alt::{Service, ServiceFactory};
use std::future::Future;
use std::task::{Context, Poll};

/// A factory that log and print error with Debug impl
pub struct ErrorLoggerFactory<F> {
    factory: F,
}

impl<F> ErrorLoggerFactory<F> {
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl<F, Req> ServiceFactory<Req> for ErrorLoggerFactory<F>
where
    F: ServiceFactory<Req>,
    F::Service: 'static,
    F::Error: Debug,
{
    type Response = F::Response;
    type Error = F::Error;
    type Config = F::Config;
    type Service = LoggerService<F::Service>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);

        async move {
            let service = service.await?;

            Ok(LoggerService { service })
        }
    }
}

pub struct LoggerService<S> {
    service: S,
}

impl<S, Req> Service<Req> for LoggerService<S>
where
    S: Service<Req> + 'static,
    S::Error: Debug,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call<'c>(&'c self, req: Req) -> Self::Future<'c>
    where
        Req: 'c,
    {
        async move {
            self.service.call(req).await.map_err(|e| {
                log::error!("{:?}", e);
                e
            })
        }
    }
}
