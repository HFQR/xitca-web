use core::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

use futures_core::stream::Stream;

use crate::bytes::Bytes;

#[derive(Debug)]
enum RequestBodyInner {
    Completion(Body),
    None,
}

/// Buffered stream of request body chunk.
///
/// impl [Stream] trait to produce chunk as [Bytes] type in async manner.
#[derive(Debug)]
pub struct RequestBody(RequestBodyInner);

impl Default for RequestBody {
    fn default() -> Self {
        Self(RequestBodyInner::None)
    }
}

impl RequestBody {
    pub(super) fn new(body: Body) -> Self {
        RequestBody(RequestBodyInner::Completion(body))
    }
}

impl Stream for RequestBody {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<io::Result<Bytes>>> {
        match self.get_mut().0 {
            RequestBodyInner::None => Poll::Ready(None),
            RequestBodyInner::Completion(ref mut body) => Pin::new(body).poll_next(cx),
        }
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H1(body)
    }
}

pub(super) struct Body(Pin<Box<dyn Stream<Item = io::Result<Bytes>>>>);

impl Body {
    pub(super) fn _new<S>(stream: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + 'static,
    {
        Self(Box::pin(stream))
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Body")
    }
}

impl Stream for Body {
    type Item = io::Result<Bytes>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().0).poll_next(cx)
    }
}
