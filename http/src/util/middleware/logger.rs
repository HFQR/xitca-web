use std::{
    convert::Infallible,
    fmt::Debug,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
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
        Instrumented {
            task: async {
                self.service.call(req).await.map_err(|e| {
                    error!("{:?}", e);
                    e
                })
            },
            span: &self.span,
        }
    }
}

pin_project! {
    #[doc(hidden)]
    /// a copy of `tracing::Instrumented` with borrowed Span.
    pub struct Instrumented<'a, T> {
        #[pin]
        task: T,
        span: &'a Span,
    }
}

// === impl Instrumented ===

impl<T: Future> Future for Instrumented<'_, T> {
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();
        this.task.poll(cx)
    }
}

impl<S> ReadyService for LoggerService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    type ReadyFuture<'f> = S::ReadyFuture<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}
