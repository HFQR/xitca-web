use std::{convert::Infallible, error, fmt, io};

use tokio::sync::mpsc::error::SendError;

#[derive(Debug)]
pub enum Error {
    Authentication(AuthenticationError),
    Io(io::Error),
    ToDo,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Authentication(ref e) => fmt::Display::fmt(e, f),
            Self::Io(ref e) => fmt::Display::fmt(e, f),
            Self::ToDo => f.write_str("error informant is yet implemented"),
        }
    }
}

impl error::Error for Error {}

impl From<Infallible> for Error {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl<T> From<SendError<T>> for Error {
    fn from(_: SendError<T>) -> Self {
        write_zero_err()
    }
}

#[derive(Debug)]
pub enum AuthenticationError {
    MissingUserName,
    MissingPassWord,
    WrongPassWord,
}

impl fmt::Display for AuthenticationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MissingUserName => f.write_str("username is missing for authentication"),
            Self::MissingPassWord => f.write_str("password is missing for authentication"),
            Self::WrongPassWord => f.write_str("password is wrong for authentication"),
        }
    }
}

impl From<AuthenticationError> for Error {
    fn from(e: AuthenticationError) -> Self {
        Self::Authentication(e)
    }
}

#[cold]
#[inline(never)]
pub(crate) fn unexpected_eof_err() -> Error {
    Error::from(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "zero byte read. remote close connection unexpectedly",
    ))
}

#[cold]
#[inline(never)]
pub(crate) fn write_zero_err() -> Error {
    Error::from(io::Error::new(
        io::ErrorKind::WriteZero,
        "zero byte written. remote close connection unexpectedly",
    ))
}
