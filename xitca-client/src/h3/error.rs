use std::{error, io};

use h3_quinn::quinn::{ConnectError, ConnectionError};
use xitca_http::error::BodyError;

#[derive(Debug)]
pub enum Error {
    Std(Box<dyn error::Error + Send + Sync>),
    Io(io::Error),
    Body(BodyError),
    H3(h3::error::Error),
    H3Connect(ConnectError),
    H3Connection(ConnectionError),
}

impl From<h3::error::Error> for Error {
    fn from(e: h3::error::Error) -> Self {
        Self::H3(e)
    }
}

impl From<ConnectError> for Error {
    fn from(e: ConnectError) -> Self {
        Self::H3Connect(e)
    }
}

impl From<ConnectionError> for Error {
    fn from(e: ConnectionError) -> Self {
        Self::H3Connection(e)
    }
}

impl From<BodyError> for Error {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}
