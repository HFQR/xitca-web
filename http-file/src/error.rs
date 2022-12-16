use core::fmt;

use std::{error, io};

#[derive(Debug)]
pub enum ServeError {
    MethodNotAllowed,
    InvalidPath,
    Io(io::Error),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MethodNotAllowed => f.write_str("request method not allowed"),
            Self::InvalidPath => f.write_str("file path is not valid"),
            Self::Io(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl error::Error for ServeError {}

impl From<io::Error> for ServeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
