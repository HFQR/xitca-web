use std::convert::Infallible;

/// Helper trait for convert Service::Error type to Service::Response.
pub trait ResponseError<Req, Res> {
    fn response_error(self, req: &mut Req) -> Res;
}

impl<Req, Res, E> ResponseError<Req, Res> for Result<Res, E>
where
    E: ResponseError<Req, Res>,
{
    fn response_error(self, req: &mut Req) -> Res {
        match self {
            Ok(res) => res,
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
    use crate::{
        body::ResponseBody,
        bytes::Bytes,
        http::{status::StatusCode, Response},
    };

    pub fn header_too_large<B>() -> Response<ResponseBody<B>> {
        status_only(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE)
    }

    pub fn bad_request<B>() -> Response<ResponseBody<B>> {
        status_only(StatusCode::BAD_REQUEST)
    }

    fn status_only<B>(status: StatusCode) -> Response<ResponseBody<B>> {
        Response::builder().status(status).body(Bytes::new().into()).unwrap()
    }
}
