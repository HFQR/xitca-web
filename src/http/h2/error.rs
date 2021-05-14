use h2::Error;

use crate::http::error::{BodyError, HttpServiceError};

impl From<Error> for BodyError {
    fn from(e: Error) -> Self {
        Self::H2(e)
    }
}

impl From<Error> for HttpServiceError {
    fn from(e: Error) -> Self {
        Self::H2(e)
    }
}
