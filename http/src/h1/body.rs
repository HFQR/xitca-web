use core::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

use futures_core::stream::Stream;

use crate::bytes::Bytes;

/// Buffered stream of request body chunk.
///
/// impl [Stream] trait to produce chunk as [Bytes] type in async manner.
pub struct RequestBody(Option<Body>);

impl fmt::Debug for RequestBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RequestBody")
    }
}

type Body = Pin<Box<dyn Stream<Item = io::Result<Bytes>>>>;

impl Default for RequestBody {
    fn default() -> Self {
        Self::none()
    }
}

impl RequestBody {
    pub(super) fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + 'static,
    {
        RequestBody(Some(Box::pin(stream)))
    }

    pub(super) fn none() -> Self {
        Self(None)
    }
}

impl Stream for RequestBody {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<io::Result<Bytes>>> {
        match self.get_mut().0 {
            Some(ref mut body) => body.as_mut().poll_next(cx),
            None => Poll::Ready(None),
        }
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H1(body)
    }
}
