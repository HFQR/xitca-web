use core::fmt;

use http::{request::Parts, Request, Response, StatusCode};
use std::{error, io};

/// high level error types for serving file.
/// see [into_response_from] and [into_response] for way of converting error to [Response] type.
///
/// [into_response_from]: ServeError::into_response_from
/// [into_response]: ServeError::into_response
#[derive(Debug)]
pub enum ServeError {
    MethodNotAllowed,
    InvalidPath,
    NotModified,
    PreconditionFailed,
    NotFound,
    Io(io::Error),
}

impl ServeError {
    /// produce a response from error. an existing request is received for inherent it's extension
    /// data and reuse it's heap allocation.
    ///
    /// # Examples
    /// ```rust
    /// # use http::Request;
    /// # use http_file::ServeError;
    /// struct User;
    ///
    /// let mut req = Request::new(());
    /// req.extensions_mut().insert(User); // data type were inserted into request extension.
    ///
    /// let mut res = ServeError::NotFound.into_response_from(req);
    ///
    /// res.extensions_mut().remove::<User>().unwrap(); // data type is moved to response.
    /// ```
    pub fn into_response_from<Ext>(self, req: Request<Ext>) -> Response<()> {
        let (
            Parts {
                mut headers,
                extensions,
                ..
            },
            _,
        ) = req.into_parts();
        headers.clear();

        let mut res = Response::new(());
        *res.headers_mut() = headers;
        *res.extensions_mut() = extensions;

        self._into_response(res)
    }

    /// produce a response from error.
    #[inline]
    pub fn into_response(self) -> Response<()> {
        self._into_response(Response::new(()))
    }

    // FIXME: handle error status.
    fn _into_response(self, mut res: Response<()>) -> Response<()> {
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res
    }
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MethodNotAllowed => f.write_str("request method not allowed"),
            Self::InvalidPath => f.write_str("file path is not valid"),
            Self::NotModified => f.write_str("file has not been modified"),
            Self::PreconditionFailed => f.write_str("precondition failed. file has been modified"),
            Self::NotFound => f.write_str("file can not be found"),
            Self::Io(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl error::Error for ServeError {}

impl From<io::Error> for ServeError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            _ => Self::Io(e),
        }
    }
}
