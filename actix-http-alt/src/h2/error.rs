use crate::error::{BodyError, HttpServiceError};

#[derive(Debug)]
pub enum Error {
    // error from h2 crate.
    H2(::h2::Error),
    Body(BodyError),
}

impl From<::h2::Error> for Error {
    fn from(e: ::h2::Error) -> Self {
        Self::H2(e)
    }
}

impl From<BodyError> for Error {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}

impl From<Error> for HttpServiceError {
    fn from(e: Error) -> Self {
        Self::H2(e)
    }
}

impl From<::h2::Error> for HttpServiceError {
    fn from(e: ::h2::Error) -> Self {
        Self::H2(Error::H2(e))
    }
}

impl From<::h2::Error> for BodyError {
    fn from(e: ::h2::Error) -> Self {
        if e.is_io() {
            Self::Io(e.into_io().unwrap())
        } else {
            Self::Std(Box::new(e))
        }
    }
}
