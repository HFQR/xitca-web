use ::h3::error::{ConnectionError as H3ConnectionError, StreamError};
use h3_quinn::quinn::ConnectionError;

use crate::error::HttpServiceError;

#[derive(Debug)]
pub enum Error<S, B> {
    Service(S),
    Body(B),
    Stream(StreamError),
    QuinnConnection(ConnectionError),
    H3Connection(H3ConnectionError),
}

impl<S, B> From<StreamError> for Error<S, B> {
    fn from(e: StreamError) -> Self {
        Self::Stream(e)
    }
}

impl<S, B> From<ConnectionError> for Error<S, B> {
    fn from(e: ConnectionError) -> Self {
        Self::QuinnConnection(e)
    }
}

impl<S, B> From<H3ConnectionError> for Error<S, B> {
    fn from(e: H3ConnectionError) -> Self {
        Self::H3Connection(e)
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
