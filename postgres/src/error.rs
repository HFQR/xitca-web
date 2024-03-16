use core::{
    convert::Infallible,
    fmt, mem,
    ops::{Deref, DerefMut},
};

use std::{error, io};

use tokio::sync::mpsc::error::SendError;
use xitca_io::bytes::BytesMut;

use crate::driver::codec::Request;

use super::from_sql::FromSqlError;

/// public facing error type. providing basic format and display based error handling.
/// for typed based error handling runtime type cast is needed with the help of other
/// public error types offered by this module.
///
/// # Example
/// ```rust
/// use xitca_postgres::error::{DriverDown, Error};
///
/// fn is_driver_down(e: Error) -> bool {
///     // downcast error to DriverDown error type to check if client driver is gone.
///     e.downcast_ref::<DriverDown>().is_some()
/// }
/// ```
pub struct Error(Box<dyn error::Error + Send + Sync>);

impl Error {
    pub(crate) fn todo() -> Self {
        Self("WIP error type placeholder".to_string().into())
    }

    pub(crate) fn unexpected() -> Self {
        Self(Box::new(UnexpectedMessage))
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn if_driver_down(&mut self) -> Option<DriverDown> {
        self.0.downcast_mut().map(mem::take)
    }
}

impl Deref for Error {
    type Target = dyn error::Error + Send + Sync;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for Error {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0.source()
    }
}

/// error indicate [Client]'s [Driver] is dropped and can't be accessed anymore.
///
/// the field inside error contains the raw bytes buffer of query message that are ready to be
/// sent to the [Driver] for transporting. It's possible to construct a new [Client] and [Driver]
/// pair where the bytes buffer could be reused for graceful retry of previous failed queries.
///
/// [Client]: crate::client::Client
/// [Driver]: crate::driver::Driver
#[derive(Default)]
pub struct DriverDown(pub BytesMut);

impl fmt::Debug for DriverDown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DriverDown").finish()
    }
}

impl fmt::Display for DriverDown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Driver is dropped and unaccessible.")
    }
}

impl error::Error for DriverDown {}

impl From<DriverDown> for Error {
    fn from(e: DriverDown) -> Self {
        Self(Box::new(e))
    }
}

pub struct InvalidColumnIndex(pub String);

impl fmt::Debug for InvalidColumnIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvalidColumnIndex").finish()
    }
}

impl fmt::Display for InvalidColumnIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid column index: {}", self.0)
    }
}

impl error::Error for InvalidColumnIndex {}

impl From<InvalidColumnIndex> for Error {
    fn from(e: InvalidColumnIndex) -> Self {
        Self(Box::new(e))
    }
}

impl From<Infallible> for Error {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self(Box::new(e))
    }
}

impl From<FromSqlError> for Error {
    fn from(e: FromSqlError) -> Self {
        Self(e)
    }
}

impl From<SendError<BytesMut>> for Error {
    fn from(e: SendError<BytesMut>) -> Self {
        Self(Box::new(DriverDown(e.0)))
    }
}

impl From<SendError<Request>> for Error {
    fn from(e: SendError<Request>) -> Self {
        Self(Box::new(DriverDown(e.0.msg)))
    }
}

/// error happens when library user failed to provide valid authentication info to database server.
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

impl error::Error for AuthenticationError {}

impl From<AuthenticationError> for Error {
    fn from(e: AuthenticationError) -> Self {
        Self(Box::new(e))
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

impl error::Error for FeatureError {}

impl From<FeatureError> for Error {
    fn from(e: FeatureError) -> Self {
        Self(Box::new(e))
    }
}

#[derive(Debug)]
pub struct UnexpectedMessage;

impl fmt::Display for UnexpectedMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unexpected message from database")
    }
}

impl error::Error for UnexpectedMessage {}

#[cold]
#[inline(never)]
pub(crate) fn unexpected_eof_err() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "zero byte read. remote close connection unexpectedly",
    )
}
