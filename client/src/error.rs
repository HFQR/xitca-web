//! strongly typed library error.

use std::{convert::Infallible, error, fmt, io, str};

use super::http::{uri, StatusCode};

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io(io::Error),
    Std(Box<dyn error::Error + Send + Sync>),
    InvalidUri(InvalidUri),
    #[cfg(feature = "http1")]
    H1(crate::h1::Error),
    #[cfg(feature = "http2")]
    H2(crate::h2::Error),
    #[cfg(feature = "http3")]
    H3(crate::h3::Error),
    #[cfg(feature = "openssl")]
    Openssl(_openssl::OpensslError),
    #[cfg(any(feature = "rustls", feature = "rustls-ring-crypto"))]
    Rustls(_rustls::RustlsError),
    Parse(ParseError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Box<dyn error::Error + Send + Sync>> for Error {
    fn from(e: Box<dyn error::Error + Send + Sync>) -> Self {
        Self::Std(e)
    }
}

impl From<str::Utf8Error> for Error {
    fn from(e: str::Utf8Error) -> Self {
        Self::Parse(ParseError::String(e))
    }
}

impl From<Infallible> for Error {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

/// a collection of multiple errors chained together.
#[derive(Debug)]
pub struct ErrorMultiple(Vec<Error>);

impl fmt::Display for ErrorMultiple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for e in self.0.iter() {
            write!(f, "{}", e)?;
        }

        Ok(())
    }
}

impl error::Error for ErrorMultiple {}

impl From<Vec<Error>> for Error {
    fn from(err: Vec<Error>) -> Self {
        Self::Std(Box::new(ErrorMultiple(err)))
    }
}

#[derive(Debug)]
pub enum InvalidUri {
    MissingHost,
    MissingScheme,
    MissingAuthority,
    MissingPathQuery,
    UnknownScheme,
    Other(uri::InvalidUri),
}

impl From<uri::InvalidUri> for InvalidUri {
    fn from(uri: uri::InvalidUri) -> Self {
        Self::Other(uri)
    }
}

impl From<uri::InvalidUri> for Error {
    fn from(e: uri::InvalidUri) -> Self {
        Self::InvalidUri(e.into())
    }
}

impl From<InvalidUri> for Error {
    fn from(e: InvalidUri) -> Self {
        Self::InvalidUri(e)
    }
}

#[derive(Debug)]
pub enum TimeoutError {
    Resolve,
    Connect,
    TlsHandshake,
    Request,
    Response,
}

impl fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve => f.write_str("dns look up timeout"),
            Self::Connect => f.write_str("socket connect timeout"),
            Self::TlsHandshake => f.write_str("tls handshake timeout"),
            Self::Request => f.write_str("request sending timeout"),
            Self::Response => f.write_str("response receiving timeout"),
        }
    }
}

impl error::Error for TimeoutError {}

impl From<TimeoutError> for Error {
    fn from(e: TimeoutError) -> Self {
        Self::Std(Box::new(e))
    }
}

#[derive(Debug)]
pub struct ResolveError {
    domain: String,
}

impl ResolveError {
    pub(crate) fn new(domain: impl Into<String>) -> Self {
        Self { domain: domain.into() }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "domain: {} can't be resolved to socket address", self.domain)
    }
}

impl error::Error for ResolveError {}

impl From<ResolveError> for Error {
    fn from(e: ResolveError) -> Self {
        Self::Std(Box::new(e))
    }
}

#[derive(Debug)]
pub enum ParseError {
    String(str::Utf8Error),
    #[cfg(feature = "json")]
    Json(serde_json::Error),
    #[cfg(feature = "websocket")]
    WebSocket(http_ws::ProtocolError),
}

#[cfg(feature = "websocket")]
impl From<http_ws::ProtocolError> for Error {
    fn from(e: http_ws::ProtocolError) -> Self {
        Self::Parse(ParseError::WebSocket(e))
    }
}

#[cfg(feature = "openssl")]
mod _openssl {
    use super::Error;

    use xitca_tls::openssl;

    pub type OpensslError = openssl::ssl::Error;

    impl From<openssl::error::ErrorStack> for Error {
        fn from(e: openssl::error::ErrorStack) -> Self {
            Self::Openssl(e.into())
        }
    }

    impl From<openssl::Error> for Error {
        fn from(e: openssl::Error) -> Self {
            match e {
                openssl::Error::Tls(e) => Self::Openssl(e),
                openssl::Error::Io(e) => Self::Io(e),
            }
        }
    }
}

#[cfg(any(feature = "rustls", feature = "rustls-ring-crypto"))]
pub(crate) use _rustls::*;

#[cfg(any(feature = "rustls", feature = "rustls-ring-crypto"))]
mod _rustls {
    use super::{io, Error};

    #[derive(Debug)]
    pub enum RustlsError {
        InvalidDnsName,
        Io(io::Error),
    }

    impl From<RustlsError> for Error {
        fn from(e: RustlsError) -> Self {
            Self::Rustls(e)
        }
    }
}

#[cfg(feature = "json")]
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(ParseError::Json(e))
    }
}

#[cfg(feature = "http1")]
impl From<crate::h1::Error> for Error {
    fn from(e: crate::h1::Error) -> Self {
        Self::H1(e)
    }
}

#[cfg(feature = "http2")]
impl From<crate::h2::Error> for Error {
    fn from(e: crate::h2::Error) -> Self {
        Self::H2(e)
    }
}

#[cfg(feature = "http3")]
impl From<crate::h3::Error> for Error {
    fn from(e: crate::h3::Error) -> Self {
        Self::H3(e)
    }
}

/// error type for unexpected http response.
/// high level crate types like http tunnel and websocket needs specific http response to function
/// properly and an unexpected http response will be converted into this error type.
#[derive(Debug)]
pub struct ErrorResponse {
    pub expect_status: StatusCode,
    pub status: StatusCode,
    pub description: &'static str,
}

impl fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "expecting response with status code {}, got {} instead. {}",
            self.expect_status, self.status, self.description
        )
    }
}

impl error::Error for ErrorResponse {}

impl From<ErrorResponse> for Error {
    fn from(e: ErrorResponse) -> Self {
        Self::Std(Box::new(e))
    }
}

#[derive(Debug)]
pub enum FeatureError {
    Http1NotEnabled,
    Http2NotEnabled,
    Http3NotEnabled,
    TlsNotEnabled,
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http1NotEnabled => f.write_str("http1")?,
            Self::Http2NotEnabled => f.write_str("http2")?,
            Self::Http3NotEnabled => f.write_str("http3")?,
            Self::TlsNotEnabled => f.write_str("openssl or rustls")?,
        };
        f.write_str(" crate feature is not enabled")
    }
}

impl error::Error for FeatureError {}

impl From<FeatureError> for Error {
    fn from(e: FeatureError) -> Self {
        Self::Std(Box::new(e))
    }
}
