use std::{fmt::Debug, future::Future};

use tracing::{error, span, Level, Span};
use xitca_service::{Service, Transform};

/// A factory for logger service.
#[derive(Clone)]
pub struct Logger {
    span: Span,
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

impl Logger {
    pub fn new() -> Self {
        Self {
            span: span!(Level::TRACE, "xitca-logger"),
        }
    }
}

impl<S, Req> Transform<S, Req> for Logger
where
    S: Service<Req>,
    S::Error: Debug,
{
    type Response = S::Response;
    type Error = S::Error;
    type Transform = LoggerService<S>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let span = self.span.clone();
        async move { Ok(LoggerService { service, span }) }
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
    S: Service<Req>,
    S::Error: Debug,
{
    type Response = S::Response;
    type Error = S::Error;
    type Ready<'f>
    where
        S: 'f,
    = S::Ready<'f>;
    type Future<'f>
    where
        S: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self.service.ready()
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let _enter = self.span.enter();
            self.service.call(req).await.map_err(|e| {
                error!("{:?}", e);
                e
            })
        }
    }
}
