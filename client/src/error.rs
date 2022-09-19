use std::{convert::Infallible, error, fmt, io, str};

use xitca_http::{error::BodyError, http::uri};

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io(io::Error),
    Std(Box<dyn error::Error + Send + Sync>),
    InvalidUri(InvalidUri),
    Resolve,
    Timeout(TimeoutError),
    TlsNotEnabled,
    Body(BodyError),
    H1(crate::h1::Error),
    #[cfg(feature = "http2")]
    H2(crate::h2::Error),
    #[cfg(feature = "http3")]
    H3(crate::h3::Error),
    #[cfg(feature = "openssl")]
    Openssl(_openssl::OpensslError),
    #[cfg(feature = "rustls")]
    Rustls(_rustls::RustlsError),
    Parse(ParseError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
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

impl From<BodyError> for Error {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}

impl From<crate::h1::Error> for Error {
    fn from(e: crate::h1::Error) -> Self {
        Self::H1(e)
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

impl From<TimeoutError> for Error {
    fn from(e: TimeoutError) -> Self {
        Self::Timeout(e)
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

    use openssl_crate::{error, ssl};

    #[derive(Debug)]
    pub enum OpensslError {
        Single(error::Error),
        Stack(error::ErrorStack),
        Ssl(ssl::Error),
    }

    impl From<error::Error> for Error {
        fn from(e: error::Error) -> Self {
            Self::Openssl(OpensslError::Single(e))
        }
    }

    impl From<error::ErrorStack> for Error {
        fn from(e: error::ErrorStack) -> Self {
            Self::Openssl(OpensslError::Stack(e))
        }
    }

    impl From<ssl::Error> for Error {
        fn from(e: ssl::Error) -> Self {
            Self::Openssl(OpensslError::Ssl(e))
        }
    }
}

#[cfg(feature = "rustls")]
pub(crate) use _rustls::*;

#[cfg(feature = "rustls")]
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
