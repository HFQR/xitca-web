use std::{
    error::Error,
    fmt::{self, Debug, Display, Formatter},
    io,
};

use actix_server_alt::net::Protocol;
use log::error;

/// HttpService layer error.
pub enum HttpServiceError {
    Ignored,
    ServiceReady,
    Timeout(TimeoutError),
    UnknownProtocol(Protocol),
    Body(BodyError),
    #[cfg(feature = "openssl")]
    Openssl(super::tls::openssl::OpensslError),
    #[cfg(feature = "rustls")]
    Rustls(super::tls::rustls::RustlsError),
    #[cfg(feature = "http1")]
    H1(super::h1::Error),
    // Http/2 error happen in HttpService handle.
    #[cfg(feature = "http2")]
    H2(h2::Error),
    // Http/3 error happen in HttpService handle.
    #[cfg(feature = "http3")]
    H3(super::h3::Error),
}

#[derive(Debug)]
pub enum TimeoutError {
    TlsAccept,
    #[cfg(feature = "http2")]
    H2Handshake,
}

impl Debug for HttpServiceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Ignored => write!(f, "Error detail is ignored."),
            Self::ServiceReady => write!(f, "Service is not ready"),
            Self::Timeout(ref timeout) => write!(f, "{:?} is timed out", timeout),
            Self::UnknownProtocol(ref protocol) => write!(f, "Protocol: {:?} is not supported", protocol),
            Self::Body(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "openssl")]
            Self::Openssl(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "http1")]
            Self::H1(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "http2")]
            Self::H2(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "http3")]
            Self::H3(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl HttpServiceError {
    pub fn log(self) {
        // TODO: add logging for different error types.
        error!("HttpService Error: {:?}", self);
    }
}

/// Request/Response body layer error.
pub enum BodyError {
    Std(Box<dyn Error>),
    Io(io::Error),
    // Http/2 error happens when handling body.
    #[cfg(feature = "http2")]
    H2(h2::Error),
    #[cfg(feature = "http3")]
    H3(super::h3::Error),
}

impl Debug for BodyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Std(ref e) => write!(f, "{:?}", e),
            Self::Io(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "http2")]
            Self::H2(ref e) => write!(f, "{:?}", e),
            #[cfg(feature = "http3")]
            Self::H3(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl Display for BodyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Std(ref e) => write!(f, "{}", e),
            Self::Io(ref e) => write!(f, "{}", e),
            #[cfg(feature = "http2")]
            Self::H2(ref e) => write!(f, "{}", e),
            #[cfg(feature = "http3")]
            Self::H3(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl Error for BodyError {}

impl From<io::Error> for BodyError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Box<dyn Error>> for BodyError {
    fn from(e: Box<dyn Error>) -> Self {
        Self::Std(e)
    }
}

impl From<BodyError> for HttpServiceError {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}

impl From<()> for HttpServiceError {
    fn from(_: ()) -> Self {
        Self::Ignored
    }
}
