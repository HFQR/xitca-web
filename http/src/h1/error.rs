use std::{fmt, io};

use crate::{error::HttpServiceError, util::timer::KeepAliveExpired};

use super::proto::error::ProtoError;

pub enum Error<S, B> {
    /// KeepAlive error should be treated as success and transform to Ok(())
    KeepAliveExpire,
    /// Closed error should be treated as success and transform to Ok(())
    Closed,
    Service(S),
    Body(B),
    Io(io::Error),
    Proto(ProtoError),
}

impl<S, B> fmt::Debug for Error<S, B>
where
    S: fmt::Debug,
    B: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::KeepAliveExpire => write!(f, "Keep-Alive time expired"),
            Self::Closed => write!(f, "Closed"),
            Self::Service(ref e) => fmt::Debug::fmt(e, f),
            Self::Body(ref e) => fmt::Debug::fmt(e, f),
            Self::Io(ref e) => fmt::Debug::fmt(e, f),
            Self::Proto(ref e) => fmt::Debug::fmt(e, f),
        }
    }
}

impl<S, B> From<ProtoError> for Error<S, B> {
    fn from(e: ProtoError) -> Self {
        Self::Proto(e)
    }
}

impl<S, B> From<io::Error> for Error<S, B> {
    fn from(e: io::Error) -> Self {
        use io::ErrorKind;
        match e.kind() {
            ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof | ErrorKind::WriteZero => Self::Closed,
            ErrorKind::WouldBlock => panic!("WouldBlock error should never be treated as error."),
            _ => Self::Io(e),
        }
    }
}

impl<S, B> From<KeepAliveExpired> for Error<S, B> {
    fn from(_: KeepAliveExpired) -> Self {
        Self::KeepAliveExpire
    }
}

impl<S, B> From<Error<S, B>> for HttpServiceError<S, B> {
    fn from(e: Error<S, B>) -> Self {
        match e {
            Error::Service(e) => HttpServiceError::Service(e),
            e => Self::H1(e),
        }
    }
}
