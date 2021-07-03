use h3_quinn::quinn::ConnectionError;

use crate::error::{BodyError, HttpServiceError};

#[derive(Debug)]
pub enum Error {
    Connection(ConnectionError),
    // error from h3 crate.
    H3(::h3::Error),
    Body(BodyError),
}

impl From<::h3::Error> for Error {
    fn from(e: ::h3::Error) -> Self {
        Self::H3(e)
    }
}

impl From<ConnectionError> for Error {
    fn from(e: ConnectionError) -> Self {
        Self::Connection(e)
    }
}

impl From<BodyError> for Error {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}

impl<E> From<Error> for HttpServiceError<E> {
    fn from(e: Error) -> Self {
        Self::H3(e)
    }
}

impl From<::h3::Error> for BodyError {
    fn from(e: ::h3::Error) -> Self {
        BodyError::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}
