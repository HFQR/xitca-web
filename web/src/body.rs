use std::error;

use futures_core::stream::Stream;

pub use xitca_http::body::{BoxStream, RequestBody, ResponseBody};

/// A extended trait for [Stream] that specify additional type info of the [Stream::Item] type.
pub trait BodyStream: Stream<Item = Result<Self::Chunk, Self::Error>> {
    type Chunk: AsRef<[u8]> + 'static;
    type Error: error::Error + 'static;
}

impl<S, T, E> BodyStream for S
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
    E: error::Error + 'static,
{
    type Chunk = T;
    type Error = E;
}
