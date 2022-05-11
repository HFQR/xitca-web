#[cfg(feature = "http1")]
pub(super) use h1_impl::*;

#[cfg(feature = "http1")]
mod h1_impl {
    use crate::{
        body::Once,
        bytes::Bytes,
        http::{status::StatusCode, Response},
    };

    pub fn header_too_large() -> Response<Once<Bytes>> {
        status_only(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE)
    }

    pub fn bad_request() -> Response<Once<Bytes>> {
        status_only(StatusCode::BAD_REQUEST)
    }

    fn status_only(status: StatusCode) -> Response<Once<Bytes>> {
        Response::builder()
            .status(status)
            .body(Once::new(Bytes::new()))
            .unwrap()
    }
}
