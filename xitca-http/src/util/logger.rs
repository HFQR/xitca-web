use std::{
    fmt::Debug,
    future::Future,
    task::{Context, Poll},
};

use tracing::{error, span, Level, Span};
use xitca_service::{Service, ServiceFactory};

/// A factory for logger service.
pub struct LoggerFactory<F> {
    factory: F,
}

impl<F> LoggerFactory<F> {
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl<F, Req> ServiceFactory<Req> for LoggerFactory<F>
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

            Ok(LoggerService {
                service,
                span: span!(Level::TRACE, "xitca_http_logger"),
            })
        }
    }
}

/// Logger service uses a tracking span called `xitca_http_logger` and would collect
/// log from all levels(from trace to info)
pub struct LoggerService<S> {
    service: S,
    span: Span,
}

impl<S, Req> Service<Req> for LoggerService<S>
where
    S: Service<Req> + 'static,
    S::Error: Debug,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let _enter = self.span.enter();
            self.service.call(req).await.map_err(|e| {
                error!(target: "xitca_http_error", "{:?}", e);
                e
            })
        }
    }
}
