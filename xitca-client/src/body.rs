use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::Stream;

/// An empty body type that always yield None on first [Stream::poll_next]
pub struct EmptyBody;

impl Stream for EmptyBody {
    type Item = Result<Vec<u8>, Infallible>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(None)
    }
}
