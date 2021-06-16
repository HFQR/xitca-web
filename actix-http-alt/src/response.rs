use std::{
    error, fmt,
    io::{self, Write},
};

use bytes::{Bytes, BytesMut};
use http::{header, status::StatusCode, Response};

use super::util::writer::Writer;

use super::body::ResponseBody;

/// Helper trait for convert Service::Error type to Service::Response.
// TODO: Add method to modify status code.
pub trait ResponseError<Res> {
    fn response_error(&mut self) -> Res;
}

impl<R, Res> ResponseError<Res> for Box<R>
where
    R: ResponseError<Res> + ?Sized,
{
    fn response_error(&mut self) -> Res {
        R::response_error(&mut **self)
    }
}

// implement ResponseError for common error types.

macro_rules! internal_impl {
    ($ty: ty) => {
        impl<B> ResponseError<Response<ResponseBody<B>>> for $ty
        where
            Self: fmt::Debug + fmt::Display,
        {
            fn response_error(&mut self) -> Response<ResponseBody<B>> {
                let mut bytes = BytesMut::new();
                write!(Writer::new(&mut bytes), "{}", self).unwrap();
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header(
                        header::CONTENT_TYPE,
                        header::HeaderValue::from_static("text/plain; charset=utf-8"),
                    )
                    .body(bytes.freeze().into())
                    .unwrap()
            }
        }
    };
}

internal_impl!(Box<dyn error::Error>);
internal_impl!(Box<dyn error::Error + Send>);
internal_impl!(Box<dyn error::Error + Send + Sync>);
internal_impl!(io::Error);

#[cfg(feature = "http1")]
pub(super) fn header_too_large<B>() -> Response<ResponseBody<B>> {
    Response::builder()
        .status(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE)
        .body(Bytes::new().into())
        .unwrap()
}
