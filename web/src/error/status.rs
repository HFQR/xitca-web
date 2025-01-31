use core::{convert::Infallible, fmt};

use std::error;

use std::backtrace::Backtrace;

use crate::{
    WebContext,
    body::ResponseBody,
    http::{StatusCode, WebResponse},
    service::Service,
};

use super::Error;

/// error type derive from http status code. produce minimal "StatusCode Reason" response and stack backtrace
/// of the location status code error occurs.
pub struct ErrorStatus {
    status: StatusCode,
    _back_trace: Backtrace,
}

impl ErrorStatus {
    /// construct an ErrorStatus type from [`StatusCode::INTERNAL_SERVER_ERROR`]
    pub fn internal() -> Self {
        // verbosity of constructor is desired here so back trace capture
        // can direct capture the call site.
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            _back_trace: Backtrace::capture(),
        }
    }

    /// construct an ErrorStatus type from [`StatusCode::BAD_REQUEST`]
    pub fn bad_request() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            _back_trace: Backtrace::capture(),
        }
    }
}

impl fmt::Debug for ErrorStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.status, f)
    }
}

impl fmt::Display for ErrorStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.status, f)
    }
}

impl error::Error for ErrorStatus {
    #[cfg(feature = "nightly")]
    fn provide<'a>(&'a self, request: &mut error::Request<'a>) {
        request.provide_ref(&self._back_trace);
    }
}

impl From<StatusCode> for ErrorStatus {
    fn from(status: StatusCode) -> Self {
        Self {
            status,
            _back_trace: Backtrace::capture(),
        }
    }
}

impl From<StatusCode> for Error {
    fn from(e: StatusCode) -> Self {
        Error::from(ErrorStatus::from(e))
    }
}

impl From<ErrorStatus> for Error {
    fn from(e: ErrorStatus) -> Self {
        Error::from_service(e)
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for ErrorStatus {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.status.call(ctx).await
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for StatusCode {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(ResponseBody::empty());
        *res.status_mut() = *self;
        Ok(res)
    }
}
