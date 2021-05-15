use std::{error, io};

use http::{header, Response};

use super::body::ResponseBody;
use bytes::Bytes;

/// Type alias for [Request](http::Response) type where generic body type
/// is default to [Body](super::body::ResponseBody)
pub type HttpResponse<B = ResponseBody> = Response<B>;

/// Helper trait for convert Service::Error type to Service::Response.
// TODO: Add method to modify status code.
pub trait ResponseError<Res> {
    fn response_error(e: Self) -> Res;
}

// implement ResponseError for common error types.

impl<B> ResponseError<HttpResponse<ResponseBody<B>>> for Box<dyn error::Error> {
    fn response_error(this: Self) -> HttpResponse<ResponseBody<B>> {
        // TODO: write this to bytes mut directly.
        let bytes = Bytes::copy_from_slice(format!("{}", this).as_bytes());
        HttpResponse::builder()
            .status(500)
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/plain; charset=utf-8"),
            )
            .body(ResponseBody::Bytes { bytes })
            .unwrap()
    }
}

impl<B> ResponseError<HttpResponse<ResponseBody<B>>> for io::Error {
    fn response_error(this: Self) -> HttpResponse<ResponseBody<B>> {
        // TODO: write this to bytes mut directly.
        let bytes = Bytes::copy_from_slice(format!("{}", this).as_bytes());
        HttpResponse::builder()
            .status(500)
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/plain; charset=utf-8"),
            )
            .body(ResponseBody::Bytes { bytes })
            .unwrap()
    }
}
