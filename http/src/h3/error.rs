use h3_quinn::quinn::ConnectionError;

use crate::error::HttpServiceError;

#[derive(Debug)]
pub enum Error<S, B> {
    Service(S),
    Body(B),
    Connection(ConnectionError),
    // error from h3 crate.
    H3(::h3::Error),
}

impl<S, B> From<::h3::Error> for Error<S, B> {
    fn from(e: ::h3::Error) -> Self {
        Self::H3(e)
    }
}

impl<S, B> From<ConnectionError> for Error<S, B> {
    fn from(e: ConnectionError) -> Self {
        Self::Connection(e)
    }
}

impl<S, B> From<Error<S, B>> for HttpServiceError<S, B> {
    fn from(e: Error<S, B>) -> Self {
        match e {
            Error::Service(e) => Self::Service(e),
            e => Self::H3(e),
        }
    }
}
