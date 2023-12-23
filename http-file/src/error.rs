use core::fmt;

use std::{error, io};

use http::{
    header::{ALLOW, CONTENT_RANGE},
    request::Parts,
    HeaderValue, Request, Response, StatusCode,
};

use super::buf::buf_write_header;

/// high level error types for serving file.
/// see [into_response_from] and [into_response] for way of converting error to [Response] type.
///
/// [into_response_from]: ServeError::into_response_from
/// [into_response]: ServeError::into_response
#[derive(Debug)]
pub enum ServeError {
    /// request method is not allowed. only GET/HEAD methods are allowed.
    MethodNotAllowed,
    /// requested file path is invalid.
    InvalidPath,
    /// requested file has not been modified.
    NotModified,
    /// requested file has been modified before given precondition time.
    PreconditionFailed,
    /// requested file range is not satisfied. u64 is the max range of file.
    RangeNotSatisfied(u64),
    /// can not find requested file.
    NotFound,
    /// I/O error from file system.
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
    /// #[derive(Clone)]
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

    fn _into_response(self, mut res: Response<()>) -> Response<()> {
        match self {
            Self::MethodNotAllowed => {
                *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
                res.headers_mut().insert(ALLOW, HeaderValue::from_static("GET,HEAD"));
            }
            Self::InvalidPath => *res.status_mut() = StatusCode::BAD_REQUEST,
            Self::NotModified => *res.status_mut() = StatusCode::NOT_MODIFIED,
            Self::PreconditionFailed => *res.status_mut() = StatusCode::PRECONDITION_FAILED,
            Self::RangeNotSatisfied(size) => {
                *res.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
                let val = buf_write_header!(0, "bytes */{size}");
                res.headers_mut().insert(CONTENT_RANGE, val);
            }
            Self::NotFound => *res.status_mut() = StatusCode::NOT_FOUND,
            Self::Io(_) => *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR,
        }
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
            Self::RangeNotSatisfied(size) => write!(f, "range is out of bound. max range of file is {size}"),
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
            io::ErrorKind::PermissionDenied => Self::InvalidPath,
            _ => Self::Io(e),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn response() {
        assert_eq!(
            ServeError::RangeNotSatisfied(128)
                .into_response()
                .headers_mut()
                .remove(CONTENT_RANGE)
                .unwrap(),
            HeaderValue::from_static("bytes */128")
        )
    }
}
