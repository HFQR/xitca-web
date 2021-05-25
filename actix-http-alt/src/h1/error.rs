use std::io;

use super::proto::ProtoError;

use crate::error::HttpServiceError;

#[derive(Debug)]
pub enum Error {
    IO(io::Error),
    Proto(ProtoError),
}

impl From<ProtoError> for Error {
    fn from(e: ProtoError) -> Self {
        Self::Proto(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::IO(e)
    }
}

impl From<Error> for HttpServiceError {
    fn from(e: Error) -> Self {
        Self::H1(e)
    }
}
