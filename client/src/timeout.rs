use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use pin_project_lite::pin_project;
use tokio::time::Sleep;

/// Configuration for various timeout setting for http client.
pub struct TimeoutConfig {
    /// Timeout for resolve DNS look up for given address.
    /// Default to 5 seconds.
    pub resolve_timeout: Duration,
    /// Timeout for establishing http connection for the first time.
    /// Default to 5 seconds.
    pub connect_timeout: Duration,
    /// Timeout for tls handshake when tls features enabled.
    /// Default to 5 seconds.
    pub tls_connect_timeout: Duration,
    /// Timeout for request reach server and response head(all lines before response body) returns.
    /// Default to 15 seconds.
    pub request_timeout: Duration,
    /// Timeout for collecting response body.
    /// Default to 15 seconds.
    pub response_timeout: Duration,
}

impl TimeoutConfig {
    pub const fn new() -> Self {
        Self {
            resolve_timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(5),
            tls_connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
            response_timeout: Duration::from_secs(15),
        }
    }
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) trait Timeout: Sized {
    fn timeout(self, timer: Pin<&mut Sleep>) -> TimeoutFuture<'_, Self> {
        TimeoutFuture { fut: self, timer }
    }
}

impl<F: Future> Timeout for F {}

pin_project! {
    pub(crate) struct TimeoutFuture<'a, F> {
        #[pin]
        fut: F,
        timer: Pin<&'a mut Sleep>
    }
}

impl<F> Future for TimeoutFuture<'_, F>
where
    F: Future,
{
    type Output = Result<F::Output, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        match this.fut.poll(cx) {
            Poll::Ready(res) => Poll::Ready(Ok(res)),
            Poll::Pending => this.timer.as_mut().poll(cx).map(|_| Err(())),
        }
    }
}
