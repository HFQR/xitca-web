use core::{convert::Infallible, fmt};

use alloc::string::String;

use std::{error, io};

use tokio::sync::mpsc::error::SendError;

use super::from_sql::FromSqlError;

#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    Feature(FeatureError),
    Authentication(AuthenticationError),
    UnexpectedMessage,
    Io(io::Error),
    FromSql(FromSqlError),
    InvalidColumnIndex(String),
    ToDo,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Feature(ref e) => fmt::Display::fmt(e, f),
            Self::Authentication(ref e) => fmt::Display::fmt(e, f),
            Self::UnexpectedMessage => f.write_str("unexpected message from server"),
            Self::Io(ref e) => fmt::Display::fmt(e, f),
            Self::FromSql(ref e) => fmt::Display::fmt(e, f),
            Self::InvalidColumnIndex(ref name) => write!(f, "invalid column {name}"),
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

impl From<FromSqlError> for Error {
    fn from(e: FromSqlError) -> Self {
        Self::FromSql(e)
    }
}

impl<T> From<SendError<T>> for Error {
    fn from(_: SendError<T>) -> Self {
        Error::from(write_zero_err())
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
            Self::MissingUserName => f.write_str("username is missing")?,
            Self::MissingPassWord => f.write_str("password is missing")?,
            Self::WrongPassWord => f.write_str("password is wrong")?,
        }

        f.write_str(" for authentication")
    }
}

impl From<AuthenticationError> for Error {
    fn from(e: AuthenticationError) -> Self {
        Self::Authentication(e)
    }
}

#[non_exhaustive]
#[derive(Debug)]
pub enum FeatureError {
    Tls,
    Quic,
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Tls => f.write_str("tls")?,
            Self::Quic => f.write_str("quic")?,
        }
        f.write_str(" feature is not enabled")
    }
}

impl From<FeatureError> for Error {
    fn from(e: FeatureError) -> Self {
        Self::Feature(e)
    }
}

#[cold]
#[inline(never)]
pub(crate) fn unexpected_eof_err() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "zero byte read. remote close connection unexpectedly",
    )
}

#[cold]
#[inline(never)]
pub(crate) fn write_zero_err() -> io::Error {
    io::Error::new(
        io::ErrorKind::WriteZero,
        "zero byte written. remote close connection unexpectedly",
    )
}
