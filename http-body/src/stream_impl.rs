//! [`Stream`] trait impl for types already implemented [`Body`] trait
//!
//! # IMPORTANT
//!
//! [`Stream::size_hint`] does not provide a fixed state for expressing [`SizeHint::None`].
//! Therefore this crate decides to use [`SizeHint::NO_BODY_HINT`] as mapped expression to it.

use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use futures_core::stream::Stream;

use super::{
    body::Body,
    frame::Frame,
    util::{StreamBody, StreamDataBody},
};

impl<B> Stream for StreamBody<B>
where
    B: Body,
{
    type Item = Result<Frame<B::Data>, B::Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().value.poll_frame(cx)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        Body::size_hint(&self.value).as_stream_size_hint()
    }
}

impl<B> Stream for StreamDataBody<B>
where
    B: Body,
{
    type Item = Result<B::Data, B::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let opt = match ready!(self.project().value.poll_frame(cx)) {
            Some(Ok(Frame::Data(data))) => Some(Ok(data)),
            Some(Err(e)) => Some(Err(e)),
            Some(_) | None => None,
        };
        Poll::Ready(opt)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.value.size_hint().as_stream_size_hint()
    }
}
