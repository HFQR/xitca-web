use core::fmt;

use std::io;

use crate::error::HttpServiceError;

use super::proto::error::ProtoError;

pub enum Error<S, B> {
    /// socket keep-alive timer expired.
    KeepAliveExpire,
    /// socket fail to receive a complete request head in given time window.
    RequestTimeout,
    Closed,
    /// service error. terminate connection right away.
    Service(S),
    /// service response body error. terminate connection right away.
    Body(B),
    /// socket and/or runtime error. terminate connection right away.
    Io(io::Error),
    /// http/1 protocol error. transform into http response and send to client.
    /// after which the connection can be gracefully shutdown or kept open.
    Proto(ProtoError),
}

impl<S, B> fmt::Debug for Error<S, B>
where
    S: fmt::Debug,
    B: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::KeepAliveExpire => f.write_str("Keep-Alive time expired"),
            Self::RequestTimeout => f.write_str("request head time out"),
            Self::Closed => f.write_str("closed"),
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
            ErrorKind::WouldBlock | ErrorKind::Interrupted => {
                unreachable!("non-blocking I/O must not emit WouldBlock/Interrupted error")
            }
            _ => Self::Io(e),
        }
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
