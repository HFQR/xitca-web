use h3_quinn::quinn::ConnectionError;

use crate::error::{BodyError, HttpServiceError};

#[derive(Debug)]
pub enum Error<E> {
    Service(E),
    Connection(ConnectionError),
    // error from h3 crate.
    H3(::h3::Error),
    Body(BodyError),
}

impl<E> From<::h3::Error> for Error<E> {
    fn from(e: ::h3::Error) -> Self {
        Self::H3(e)
    }
}

impl<E> From<ConnectionError> for Error<E> {
    fn from(e: ConnectionError) -> Self {
        Self::Connection(e)
    }
}

impl<E> From<BodyError> for Error<E> {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}

impl<E> From<Error<E>> for HttpServiceError<E> {
    fn from(e: Error<E>) -> Self {
        match e {
            Error::Service(e) => Self::Service(e),
            e => Self::H3(e),
        }
    }
}

impl From<::h3::Error> for BodyError {
    fn from(e: ::h3::Error) -> Self {
        BodyError::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}
