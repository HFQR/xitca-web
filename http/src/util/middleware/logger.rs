use std::{
    convert::Infallible,
    fmt::Debug,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use tracing::{error, span, Level, Span};
use xitca_service::{ready::ReadyService, Service};

/// middleware for [tracing] logging system.
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
    /// enable middleware with default setting.
    pub fn new() -> Self {
        Self::with_span(span!(Level::TRACE, "xitca-http-logger"))
    }

    /// enable middleware with given [Span].
    pub fn with_span(span: Span) -> Self {
        Self { span }
    }
}

impl<S> Service<S> for Logger {
    type Response = LoggerService<S>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f;

    fn call<'s>(&'s self, service: S) -> Self::Future<'s>
    where
        S: 's,
    {
        let span = self.span.clone();
        async { Ok(LoggerService { service, span }) }
    }
}

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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f, Req: 'f;

    fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
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

impl<S> ReadyService for LoggerService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    type Future<'f> = S::Future<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::Future<'_> {
        self.service.ready()
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

impl<T: Future> Future for Instrumented<'_, T> {
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();
        this.task.poll(cx)
    }
}
