use std::{
    convert::Infallible,
    error::Error,
    fmt::{self, Debug, Display, Formatter},
    io,
};

use tracing::error;

use super::{http::Version, tls::TlsError};

/// HttpService layer error.
pub enum HttpServiceError<E> {
    Ignored,
    ServiceReady,
    Service(E),
    Timeout(TimeoutError),
    UnSupportedVersion(Version),
    Body(BodyError),
    Tls(TlsError),
    #[cfg(feature = "http1")]
    H1(super::h1::Error<E>),
    // Http/2 error happen in HttpService handle.
    #[cfg(feature = "http2")]
    H2(super::h2::Error<E>),
    // Http/3 error happen in HttpService handle.
    #[cfg(feature = "http3")]
    H3(super::h3::Error<E>),
}

impl<E: Debug> Debug for HttpServiceError<E> {
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

impl<E: Debug> HttpServiceError<E> {
    pub fn log(self, target: &str) {
        // TODO: add logging for different error types.
        error!(target = target, ?self);
    }
}

/// A collection of std error types. Due to xitca-http's single thread nature it support
/// a wider range of [`Error`](std::error::Error) trait object with different bounds.
pub enum StdError {
    Std(Box<dyn Error>),
    StdSend(Box<dyn Error + Send>),
    StdSendSync(Box<dyn Error + Send + Sync>),
}

impl From<Box<dyn Error>> for StdError {
    fn from(e: Box<dyn Error>) -> Self {
        Self::Std(e)
    }
}

impl From<Box<dyn Error + Send>> for StdError {
    fn from(e: Box<dyn Error + Send>) -> Self {
        Self::StdSend(e)
    }
}

impl From<Box<dyn Error + Send + Sync>> for StdError {
    fn from(e: Box<dyn Error + Send + Sync>) -> Self {
        Self::StdSendSync(e)
    }
}

impl Debug for StdError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Std(ref e) => write!(f, "{:?}", *e),
            Self::StdSend(ref e) => write!(f, "{:?}", *e),
            Self::StdSendSync(ref e) => write!(f, "{:?}", *e),
        }
    }
}

impl Display for StdError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Std(ref e) => write!(f, "{}", *e),
            Self::StdSend(ref e) => write!(f, "{}", *e),
            Self::StdSendSync(ref e) => write!(f, "{}", *e),
        }
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
pub enum BodyError {
    Std(StdError),
    Io(io::Error),
}

impl Debug for BodyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Std(ref e) => write!(f, "{:?}", e),
            Self::Io(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl Display for BodyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Std(ref e) => write!(f, "{}", e),
            Self::Io(ref e) => write!(f, "{}", e),
        }
    }
}

impl Error for BodyError {}

impl From<io::Error> for BodyError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl<E> From<E> for BodyError
where
    E: Into<StdError>,
{
    fn from(e: E) -> Self {
        Self::Std(e.into())
    }
}

impl From<Infallible> for BodyError {
    fn from(_: Infallible) -> BodyError {
        unreachable!("Infallible error should never happen")
    }
}

impl<E> From<BodyError> for HttpServiceError<E> {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}

impl<E> From<()> for HttpServiceError<E> {
    fn from(_: ()) -> Self {
        Self::Ignored
    }
}

impl<E> From<Infallible> for HttpServiceError<E> {
    fn from(_: Infallible) -> Self {
        unreachable!("Infallible error should never happen")
    }
}
