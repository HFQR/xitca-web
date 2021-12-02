use std::{error, fmt, io};

use xitca_http::{body::ResponseBody, bytes::Bytes, http::Response, ResponseError};

#[derive(Debug)]
pub enum Error {
    IsDirectory,
    Io(io::Error),
    UriSegmentError(UriSegmentError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::IsDirectory => write!(f, "Unable to render directory without index file"),
            Self::Io(ref e) => fmt::Display::fmt(e, f),
            Self::UriSegmentError(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl error::Error for Error {}

#[derive(Debug, PartialEq)]
pub enum UriSegmentError {
    /// The segment started with the wrapped invalid character.
    Start(char),
    /// The segment contained the wrapped invalid character.
    Char(char),
    /// The segment ended with the wrapped invalid character.
    End(char),
}

impl fmt::Display for UriSegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Start(ref char) => write!(
                f,
                "The segment started with the wrapped invalid character with {}",
                char
            ),
            Self::Char(ref char) => write!(f, "The segment contained the wrapped invalid character with {}", char),
            Self::End(ref char) => write!(f, "The segment ended with the wrapped invalid character with {}", char),
        }
    }
}

impl error::Error for UriSegmentError {}

impl From<UriSegmentError> for Error {
    fn from(e: UriSegmentError) -> Self {
        Self::UriSegmentError(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl<Req, B> ResponseError<Req, Response<ResponseBody<B>>> for Error {
    fn response_error(&mut self, _: &mut Req) -> Response<ResponseBody<B>> {
        Response::new(Bytes::from(format!("{}", self)).into())
    }
}
