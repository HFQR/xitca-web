use std::{convert::Infallible, fmt};

use http::{status::StatusCode, Response};

use super::{body::ResponseBody, bytes::Bytes};

/// Helper trait for convert Service::Error type to Service::Response.
pub trait ResponseError<Req, Res>: fmt::Debug {
    fn status_code() -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn response_error(self, req: &mut Req) -> Res;
}

impl<Req, Res, T, E> ResponseError<Req, Res> for Result<T, E>
where
    T: ResponseError<Req, Res>,
    E: ResponseError<Req, Res>,
{
    fn response_error(self, req: &mut Req) -> Res {
        match self {
            Ok(t) => t.response_error(req),
            Err(e) => e.response_error(req),
        }
    }
}

impl<Req, Res> ResponseError<Req, Res> for Infallible {
    fn response_error(self, _: &mut Req) -> Res {
        unreachable!()
    }
}

#[cfg(feature = "http1")]
pub(super) use h1_impl::*;

#[cfg(feature = "http1")]
mod h1_impl {
    use super::*;

    pub fn header_too_large<B>() -> Response<ResponseBody<B>> {
        status_only(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE)
    }

    pub fn bad_request<B>() -> Response<ResponseBody<B>> {
        status_only(StatusCode::BAD_REQUEST)
    }
}

fn status_only<B>(status: StatusCode) -> Response<ResponseBody<B>> {
    Response::builder().status(status).body(Bytes::new().into()).unwrap()
}
