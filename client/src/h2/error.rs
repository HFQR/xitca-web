use std::{error, io};

use xitca_http::error::BodyError;

#[derive(Debug)]
pub enum Error {
    Std(Box<dyn error::Error + Send + Sync>),
    Io(io::Error),
    Body(BodyError),
    H2(h2::Error),
}

impl From<h2::Error> for Error {
    fn from(e: h2::Error) -> Self {
        Self::H2(e)
    }
}

impl From<BodyError> for Error {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}
