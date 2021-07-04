use std::{fmt, io};

use super::proto::ProtoError;

use crate::error::{BodyError, HttpServiceError};

pub enum Error<E> {
    /// Closed error should be treated as success and transform to Ok(())
    Closed,
    Service(E),
    Body(BodyError),
    Io(io::Error),
    Proto(ProtoError),
}

impl<E: fmt::Debug> fmt::Debug for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Closed => write!(f, "Closed"),
            Self::Service(ref e) => write!(f, "{:?}", e),
            Self::Body(ref e) => write!(f, "{:?}", e),
            Self::Io(ref e) => write!(f, "{:?}", e),
            Self::Proto(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl<E> From<BodyError> for Error<E> {
    fn from(e: BodyError) -> Self {
        Self::Body(e)
    }
}

impl<E> From<ProtoError> for Error<E> {
    fn from(e: ProtoError) -> Self {
        Self::Proto(e)
    }
}

impl<E> From<io::Error> for Error<E> {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::ConnectionReset => Self::Closed,
            io::ErrorKind::WouldBlock => panic!("WouldBlock error should never be treated as error."),
            _ => Self::Io(e),
        }
    }
}

impl<E> From<Error<E>> for HttpServiceError<E> {
    fn from(e: Error<E>) -> Self {
        match e {
            Error::Service(e) => HttpServiceError::Service(e),
            e => Self::H1(e),
        }
    }
}
