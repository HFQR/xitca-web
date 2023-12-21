//! http body types and traits.

use futures_core::stream::Stream;

pub use xitca_http::body::{none_body_hint, BoxBody, RequestBody, ResponseBody, NONE_BODY_HINT};

pub(crate) use xitca_http::body::Either;

use crate::error::BodyError;

/// an extended trait for [Stream] that specify additional type info of the [Stream::Item] type.
pub trait BodyStream: Stream<Item = Result<Self::Chunk, Self::Error>> {
    type Chunk: AsRef<[u8]> + 'static;
    type Error: Into<BodyError>;
}

impl<S, T, E> BodyStream for S
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
    E: Into<BodyError>,
{
    type Chunk = T;
    type Error = E;
}
