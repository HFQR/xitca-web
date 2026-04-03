use core::{
    pin::Pin,
    task::{Context, Poll},
};
use std::ops::{Deref, DerefMut};

use super::{frame::Frame, size_hint::SizeHint};

type Result<T, E> = std::result::Result<Frame<T>, E>;

pub trait Body {
    type Data;
    type Error;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Self::Data, Self::Error>>>;

    #[inline]
    fn is_end_stream(&self) -> bool {
        false
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

impl<B> Body for Box<B>
where
    B: Body + ?Sized + Unpin,
{
    type Data = B::Data;
    type Error = B::Error;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        Body::poll_frame(Pin::new(self.get_mut().deref_mut()), cx)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.deref().is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.deref().size_hint()
    }
}

impl<B> Body for Pin<Box<B>>
where
    B: Body + ?Sized,
{
    type Data = B::Data;
    type Error = B::Error;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        Body::poll_frame(self.as_deref_mut(), cx)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.as_ref().is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.as_ref().size_hint()
    }
}
