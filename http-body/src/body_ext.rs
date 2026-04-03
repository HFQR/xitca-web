use core::{
    pin::Pin,
    task::{Context, Poll},
};

use super::{body::Body, frame::Frame};

pub trait BodyExt {
    fn frame(&mut self) -> FrameFuture<'_, Self>
    where
        Self: Body + Sized + Unpin,
    {
        FrameFuture { body: self }
    }
}

pub struct FrameFuture<'a, B> {
    body: &'a mut B,
}

impl<B> Future for FrameFuture<'_, B>
where
    B: Body + Unpin,
{
    type Output = Option<Result<Frame<B::Data>, B::Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Body::poll_frame(Pin::new(self.get_mut().body), cx)
    }
}

impl<B> BodyExt for B where B: Body {}
