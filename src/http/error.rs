use std::{
    error::Error,
    fmt::{self, Debug, Display, Formatter},
};

use log::error;

/// Service layer error that used for logging.
pub enum HttpServiceError {
    Openssl(super::tls::openssl::OpensslError),
    ServiceReady,
    H2(h2::Error),
}

impl Debug for HttpServiceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ServiceReady => write!(f, "Service is not ready"),
            Self::H2(ref e) => write!(f, "{:?}", e),
            Self::Openssl(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl HttpServiceError {
    pub fn log(self) {
        // TODO: add logging for different error types.
        error!("HttpService Error: {:?}", self);
    }
}

/// Stream body error.
pub enum BodyError {
    H2(h2::Error),
}

impl Debug for BodyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::H2(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl Display for BodyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::H2(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl Error for BodyError {}
