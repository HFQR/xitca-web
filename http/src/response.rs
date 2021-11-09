use std::{
    convert::Infallible,
    error, fmt,
    io::{self, Write},
};

use http::{header, status::StatusCode, Response};

use super::{
    body::ResponseBody,
    bytes::{Bytes, BytesMut},
    util::writer::Writer,
};

/// Helper trait for convert Service::Error type to Service::Response.
// TODO: Add method to modify status code.
pub trait ResponseError<Req, Res>: fmt::Debug {
    fn status_code() -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn response_error(&mut self, req: &mut Req) -> Res;
}

// implement ResponseError for common error types.
impl<Req, B> ResponseError<Req, Response<ResponseBody<B>>> for () {
    fn response_error(&mut self, _: &mut Req) -> Response<ResponseBody<B>> {
        status_only(<Self as ResponseError<Req, Response<ResponseBody<B>>>>::status_code())
    }
}

macro_rules! internal_impl {
    ($ty: ty) => {
        impl<B, Req> ResponseError<Req, Response<ResponseBody<B>>> for $ty {
            fn response_error(&mut self, _: &mut Req) -> Response<ResponseBody<B>> {
                let mut bytes = BytesMut::new();
                write!(Writer::new(&mut bytes), "{}", self).unwrap();
                Response::builder()
                    .status(<Self as ResponseError<Req, Response<ResponseBody<B>>>>::status_code())
                    .header(
                        header::CONTENT_TYPE,
                        header::HeaderValue::from_static("text/plain; charset=utf-8"),
                    )
                    .body(bytes.into())
                    .unwrap()
            }
        }
    };
}

internal_impl!(Box<dyn error::Error>);
internal_impl!(Box<dyn error::Error + Send>);
internal_impl!(Box<dyn error::Error + Send + Sync>);
internal_impl!(io::Error);
internal_impl!(Infallible);

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
