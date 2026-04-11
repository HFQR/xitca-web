//! error types.

use core::{convert::Infallible, error::Error, fmt};

use tracing::error;

use super::http::Version;

pub(crate) use super::tls::TlsError;

/// HttpService layer error.
pub enum HttpServiceError<S, B> {
    Ignored,
    Service(S),
    Body(B),
    Timeout(TimeoutError),
    UnSupportedVersion(Version),
    Tls(TlsError),
    #[cfg(feature = "http1")]
    H1(super::h1::Error<S, B>),
    // Http/2 error happen in HttpService handle.
    #[cfg(feature = "http2")]
    H2(super::h2::Error<S, B>),
    // Http/3 error happen in HttpService handle.
    #[cfg(feature = "http3")]
    H3(super::h3::Error<S, B>),
}

impl<S, B> fmt::Debug for HttpServiceError<S, B>
where
    S: fmt::Debug,
    B: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Ignored => f.write_str("Error detail is ignored."),
            Self::Service(ref e) => fmt::Debug::fmt(e, f),
            Self::Timeout(ref timeout) => write!(f, "{timeout:?} is timed out"),
            Self::UnSupportedVersion(ref protocol) => write!(f, "Protocol: {protocol:?} is not supported"),
            Self::Body(ref e) => fmt::Debug::fmt(e, f),
            Self::Tls(ref e) => fmt::Debug::fmt(e, f),
            #[cfg(feature = "http1")]
            Self::H1(ref e) => fmt::Debug::fmt(e, f),
            #[cfg(feature = "http2")]
            Self::H2(ref e) => fmt::Debug::fmt(e, f),
            #[cfg(feature = "http3")]
            Self::H3(ref e) => fmt::Debug::fmt(e, f),
        }
    }
}

impl<S, B> HttpServiceError<S, B>
where
    S: fmt::Debug,
    B: fmt::Debug,
{
    pub fn log(self, target: &str) {
        // TODO: add logging for different error types.
        error!(target = target, ?self);
    }
}

/// time out error from async task that run for too long.
#[derive(Debug)]
pub enum TimeoutError {
    TlsAccept,
}

impl<S, B> From<()> for HttpServiceError<S, B> {
    fn from(_: ()) -> Self {
        Self::Ignored
    }
}

impl<S, B> From<Infallible> for HttpServiceError<S, B> {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

/// Default Request/Response body error.
pub type BodyError = Box<dyn Error + Send + Sync>;
