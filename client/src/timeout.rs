use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use pin_project_lite::pin_project;
use tokio::time::Sleep;

/// Configuration for various timeout setting for http client.
#[derive(Clone)]
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

/// Configuration for various timeout setting for http request by client, all fields are optional,
/// the one not set will use the value from [TimeoutConfig] of the client.
pub struct PartialTimeoutConfig {
    /// Timeout for resolve DNS look up for given address.
    pub resolve_timeout: Option<Duration>,
    /// Timeout for establishing http connection for the first time.
    pub connect_timeout: Option<Duration>,
    /// Timeout for tls handshake when tls features enabled.
    pub tls_connect_timeout: Option<Duration>,
    /// Timeout for request reach server and response head(all lines before response body) returns.
    pub request_timeout: Option<Duration>,
    /// Timeout for collecting response body.
    pub response_timeout: Option<Duration>,
}

impl PartialTimeoutConfig {
    pub const fn new() -> Self {
        Self {
            resolve_timeout: None,
            connect_timeout: None,
            tls_connect_timeout: None,
            request_timeout: None,
            response_timeout: None,
        }
    }

    pub fn to_timeout_config(&self, client_timeout: &TimeoutConfig) -> TimeoutConfig {
        TimeoutConfig {
            resolve_timeout: self.resolve_timeout.unwrap_or(client_timeout.resolve_timeout),
            connect_timeout: self.connect_timeout.unwrap_or(client_timeout.connect_timeout),
            tls_connect_timeout: self.tls_connect_timeout.unwrap_or(client_timeout.tls_connect_timeout),
            request_timeout: self.request_timeout.unwrap_or(client_timeout.request_timeout),
            response_timeout: self.response_timeout.unwrap_or(client_timeout.response_timeout),
        }
    }
}

impl Default for PartialTimeoutConfig {
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
