use crate::error::{BodyError, HttpServiceError};

#[derive(Debug)]
pub enum Error<S, B> {
    Service(S),
    Body(B),
    // error from h2 crate.
    H2(::h2::Error),
}

impl<S, B> From<::h2::Error> for Error<S, B> {
    fn from(e: ::h2::Error) -> Self {
        Self::H2(e)
    }
}

impl<S, B> From<Error<S, B>> for HttpServiceError<S, B> {
    fn from(e: Error<S, B>) -> Self {
        match e {
            Error::Service(e) => Self::Service(e),
            e => Self::H2(e),
        }
    }
}

impl<S, B> From<::h2::Error> for HttpServiceError<S, B> {
    fn from(e: ::h2::Error) -> Self {
        Self::H2(Error::H2(e))
    }
}

impl From<::h2::Error> for BodyError {
    fn from(e: ::h2::Error) -> Self {
        BodyError::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}
