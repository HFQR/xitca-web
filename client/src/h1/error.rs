use std::{error, io};

use xitca_http::h1::proto::error::ProtoError;

#[derive(Debug)]
pub enum UnexpectedStateError {
    RemainingData,
    ConnectionClosed,
}

#[derive(Debug)]
pub enum Error {
    Std(Box<dyn error::Error + Send + Sync>),
    Io(io::Error),
    Proto(ProtoError),
    UnexpectedState(UnexpectedStateError),
}

impl From<Box<dyn error::Error + Send + Sync>> for Error {
    fn from(e: Box<dyn error::Error + Send + Sync>) -> Self {
        Self::Std(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ProtoError> for Error {
    fn from(e: ProtoError) -> Self {
        Self::Proto(e)
    }
}

impl From<UnexpectedStateError> for Error {
    fn from(e: UnexpectedStateError) -> Self {
        Self::UnexpectedState(e)
    }
}
