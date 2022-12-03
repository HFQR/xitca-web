use std::{
    convert::Infallible,
    error::Error,
    fmt::{self, Debug, Display, Formatter},
    io,
};

use tracing::error;
use xitca_service::pipeline::PipelineE;

use super::http::Version;

pub(crate) use super::tls::TlsError;

pub(crate) type BuildError<Tls, Svc> = PipelineE<Tls, Svc>;

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

impl<S, B> Debug for HttpServiceError<S, B>
where
    S: Debug,
    B: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Ignored => write!(f, "Error detail is ignored."),
            Self::Service(ref e) => Debug::fmt(e, f),
            Self::Timeout(ref timeout) => write!(f, "{timeout:?} is timed out"),
            Self::UnSupportedVersion(ref protocol) => write!(f, "Protocol: {protocol:?} is not supported"),
            Self::Body(ref e) => Debug::fmt(e, f),
            Self::Tls(ref e) => Debug::fmt(e, f),
            #[cfg(feature = "http1")]
            Self::H1(ref e) => Debug::fmt(e, f),
            #[cfg(feature = "http2")]
            Self::H2(ref e) => Debug::fmt(e, f),
            #[cfg(feature = "http3")]
            Self::H3(ref e) => Debug::fmt(e, f),
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
#[derive(Debug)]
pub struct BodyError(Box<dyn Error + Send + Sync>);

impl Display for BodyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl Error for BodyError {}

impl From<io::Error> for BodyError {
    fn from(e: io::Error) -> Self {
        Self(Box::new(e))
    }
}

impl From<Box<dyn Error + Send + Sync>> for BodyError {
    fn from(e: Box<dyn Error + Send + Sync>) -> Self {
        Self(e)
    }
}

impl From<Infallible> for BodyError {
    fn from(e: Infallible) -> BodyError {
        match e {}
    }
}
