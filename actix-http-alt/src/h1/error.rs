use std::io;

use crate::error::HttpServiceError;

#[derive(Debug)]
pub enum Error {
    IO(io::Error),
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
