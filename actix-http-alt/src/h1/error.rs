use std::io;

use super::proto::ProtoError;

use crate::error::{BodyError, HttpServiceError};

#[derive(Debug)]
pub enum Error {
    /// Closed error should be treated as success and transform to Ok(())
    Closed,
    Body(BodyError),
    Io(io::Error),
    Proto(ProtoError),
}

impl From<BodyError> for Error {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}

impl From<ProtoError> for Error {
    fn from(e: ProtoError) -> Self {
        Self::Proto(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Error> for HttpServiceError {
    fn from(e: Error) -> Self {
        Self::H1(e)
    }
}
