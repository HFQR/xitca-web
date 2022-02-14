use std::{
    convert::Infallible,
    error::Error,
    fmt::{self, Debug, Display, Formatter},
    io,
};

use tracing::error;

use super::{http::Version, tls::TlsError};

/// HttpService layer error.
pub enum HttpServiceError<S, B> {
    Ignored,
    ServiceReady,
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

impl<S, B> Debug for HttpServiceError<S, B>
where
    S: Debug,
    B: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Ignored => write!(f, "Error detail is ignored."),
            Self::ServiceReady => write!(f, "Service is not ready"),
            Self::Service(ref e) => write!(f, "{:?}", e),
            Self::Timeout(ref timeout) => write!(f, "{:?} is timed out", timeout),
            Self::UnSupportedVersion(ref protocol) => write!(f, "Protocol: {:?} is not supported", protocol),
            Self::Body(ref e) => write!(f, "{:?}", e),
            Self::Tls(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "http1")]
            Self::H1(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "http2")]
            Self::H2(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "http3")]
            Self::H3(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl<S, B> HttpServiceError<S, B>
where
    S: Debug,
    B: Debug,
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
    #[cfg(feature = "http2")]
    H2Handshake,
}

/// Request/Response body layer error.
#[derive(Debug)]
pub enum BodyError {
    Std(Box<dyn Error + Send + Sync>),
    Io(io::Error),
    OverFlow,
}

impl Display for BodyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Std(ref e) => write!(f, "{}", e),
            Self::Io(ref e) => write!(f, "{}", e),
            Self::OverFlow => write!(f, "Body length is overflow"),
        }
    }
}

impl Error for BodyError {}

impl From<io::Error> for BodyError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Box<dyn Error + Send + Sync>> for BodyError {
    fn from(e: Box<dyn Error + Send + Sync>) -> Self {
        Self::Std(e)
    }
}

impl From<Infallible> for BodyError {
    fn from(_: Infallible) -> BodyError {
        unreachable!("Infallible error should never happen")
    }
}

impl<S, B> From<()> for HttpServiceError<S, B> {
    fn from(_: ()) -> Self {
        Self::Ignored
    }
}

impl<S, B> From<Infallible> for HttpServiceError<S, B> {
    fn from(_: Infallible) -> Self {
        unreachable!("Infallible error should never happen")
    }
}
