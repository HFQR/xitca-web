use std::{error, fmt, io};

use tokio::sync::mpsc::error::SendError;

#[derive(Debug)]
pub enum Error {
    ToDo,
    ConnectionClosed,
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ToDo => write!(f, "error informant is yet implemented"),
            Self::ConnectionClosed => write!(f, "Connection is closed"),
            Self::Io(ref e) => write!(f, "{}", e),
        }
    }
}

impl error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl<T> From<SendError<T>> for Error {
    fn from(_: SendError<T>) -> Self {
        Self::ConnectionClosed
    }
}
