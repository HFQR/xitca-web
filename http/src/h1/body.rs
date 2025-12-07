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
pub struct RequestBody(Option<Pin<Box<dyn Stream<Item = io::Result<Bytes>>>>>);

impl fmt::Debug for RequestBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RequestBody")
    }
}

impl RequestBody {
    pub(super) fn new<S>(body: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + 'static,
    {
        Self(Some(Box::pin(body)))
    }
}

impl Default for RequestBody {
    fn default() -> Self {
        Self(None)
    }
}

impl Stream for RequestBody {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<io::Result<Bytes>>> {
        match self.get_mut().0 {
            None => Poll::Ready(None),
            Some(ref mut body) => Pin::new(body).poll_next(cx),
        }
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H1(body)
    }
}
