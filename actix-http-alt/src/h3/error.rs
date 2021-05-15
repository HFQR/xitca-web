use h3_quinn::quinn::ConnectionError;

use crate::error::HttpServiceError;

#[derive(Debug)]
pub enum H3Error {
    Connection(ConnectionError),
    // error from h3 crate.
    H3(::h3::Error),
}

impl From<ConnectionError> for HttpServiceError {
    fn from(e: ConnectionError) -> Self {
        Self::H3(H3Error::Connection(e))
    }
}

impl From<::h3::Error> for HttpServiceError {
    fn from(e: ::h3::Error) -> Self {
        Self::H3(H3Error::H3(e))
    }
}
