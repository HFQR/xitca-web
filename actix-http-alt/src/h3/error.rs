use h3_quinn::quinn::ConnectionError;

use crate::error::{BodyError, HttpServiceError};

#[derive(Debug)]
pub enum Error {
    Connection(ConnectionError),
    // error from h3 crate.
    H3(::h3::Error),
}

impl From<ConnectionError> for HttpServiceError {
    fn from(e: ConnectionError) -> Self {
        Self::H3(Error::Connection(e))
    }
}

impl From<::h3::Error> for HttpServiceError {
    fn from(e: ::h3::Error) -> Self {
        Self::H3(Error::H3(e))
    }
}

impl From<::h3::Error> for BodyError {
    fn from(e: ::h3::Error) -> Self {
        Self::H3(Error::H3(e))
    }
}
