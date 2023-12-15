use std::{
    fmt::Debug,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use tracing::{error, span, Level, Span};
use xitca_service::{ready::ReadyService, Service};

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

impl<S, E> Service<Result<S, E>> for Logger {
    type Response = LoggerService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| LoggerService {
            service,
            span: self.span.clone(),
        })
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

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        Instrumented {
            task: async {
                self.service.call(req).await.map_err(|e| {
                    error!("{:?}", e);
                    e
                })
            },
            span: &self.span,
        }
        .await
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

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.service.ready().await
    }
}
