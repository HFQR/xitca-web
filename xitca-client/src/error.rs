use std::io;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io(io::Error),
    InvalidUri(InvalidUri),
    Timeout(TimeoutError),
    #[cfg(feature = "openssl")]
    Openssl(_openssl::OpensslError),
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

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug)]
pub enum InvalidUri {
    ReasonUnknown,
    MissingHost,
    MissingScheme,
    MissingAuthority,
    MissingPathQuery,
    UnknownScheme,
}

impl From<http::uri::InvalidUri> for InvalidUri {
    fn from(_: http::uri::InvalidUri) -> Self {
        Self::ReasonUnknown
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
}

impl From<TimeoutError> for Error {
    fn from(e: TimeoutError) -> Self {
        Self::Timeout(e)
    }
}
