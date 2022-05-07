use std::{convert::Infallible, fmt::Debug, future::Future};

use tracing::{error, span, Level, Span};
use xitca_service::{ready::ReadyService, BuildService, Service};

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
        Self::with_span(span!(Level::TRACE, "xitca-logger"))
    }

    pub fn with_span(span: Span) -> Self {
        Self { span }
    }
}

impl<S> BuildService<S> for Logger {
    type Service = LoggerService<S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, service: S) -> Self::Future {
        let span = self.span.clone();
        async { Ok(LoggerService { service, span }) }
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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where S: 'f;

    fn call(&self, req: Req) -> Self::Future<'_> {
        use tracing::Instrument;
        async move {
            self.service.call(req).await.map_err(|e| {
                error!("{:?}", e);
                e
            })
        }
        .instrument(self.span.clone())
    }
}

impl<S, Req> ReadyService<Req> for LoggerService<S>
where
    S: ReadyService<Req>,
    S::Error: Debug,
{
    type Ready = S::Ready;

    type ReadyFuture<'f> = S::ReadyFuture<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}
