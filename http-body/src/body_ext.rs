use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use pin_project_lite::pin_project;

use super::{body::Body, frame::Frame};

pub trait BodyExt: Body {
    fn frame(&mut self) -> FrameFuture<'_, Self>
    where
        Self: Unpin,
    {
        FrameFuture { body: self }
    }

    fn data(&mut self) -> DataFuture<'_, Self>
    where
        Self: Unpin,
    {
        DataFuture { body: self }
    }

    fn chain<B>(self, other: B) -> ChainBody<Self, B>
    where
        Self: Sized,
        B: Body<Data = Self::Data, Error = Self::Error>,
    {
        ChainBody {
            first: self,
            second: other,
        }
    }
}

pub struct FrameFuture<'a, B: ?Sized> {
    body: &'a mut B,
}

impl<B> Future for FrameFuture<'_, B>
where
    B: Body + Unpin,
{
    type Output = Option<Result<Frame<B::Data>, B::Error>>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Body::poll_frame(Pin::new(self.get_mut().body), cx)
    }
}

pub struct DataFuture<'a, B: ?Sized> {
    body: &'a mut B,
}

impl<B> Future for DataFuture<'_, B>
where
    B: Body + Unpin,
{
    type Output = Option<Result<B::Data, B::Error>>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let opt = match ready!(Body::poll_frame(Pin::new(self.get_mut().body), cx)) {
            Some(Ok(Frame::Data(data))) => Some(Ok(data)),
            Some(Err(e)) => Some(Err(e)),
            Some(_) | None => None,
        };

        Poll::Ready(opt)
    }
}

pin_project! {
    pub struct ChainBody<A, B> {
        #[pin]
        first: A,
        #[pin]
        second: B
    }
}

impl<A, B> Body for ChainBody<A, B>
where
    A: Body,
    B: Body<Data = A::Data, Error = A::Error>,
{
    type Data = A::Data;
    type Error = A::Error;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();

        match ready!(this.first.poll_frame(cx)) {
            Some(frame) => Poll::Ready(Some(frame)),
            None => this.second.poll_frame(cx),
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.first.is_end_stream() && self.second.is_end_stream()
    }

    fn size_hint(&self) -> crate::SizeHint {
        self.first.size_hint() + self.second.size_hint()
    }
}

impl<B> BodyExt for B where B: Body {}
