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
