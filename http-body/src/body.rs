use core::{
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

use super::{frame::Frame, size_hint::SizeHint};

type Result<T, E> = std::result::Result<Frame<T>, E>;

/// An asynchronous streaming body composed of [`Frame`]s.
///
/// Each frame is either a [`Frame::Data`] carrying payload bytes or a [`Frame::Trailers`]
/// carrying trailing headers. The stream signals completion by returning `Poll::Ready(None)`.
pub trait Body {
    /// The payload data type yielded by [`Frame::Data`] variants.
    type Data;
    /// The error type that can occur while producing frames.
    type Error;

    /// Attempt to pull the next frame from the body.
    ///
    /// # Return values
    ///
    /// - `Poll::Pending` — the next frame is not yet available.
    /// - `Poll::Ready(Some(Ok(frame)))` — a [`Frame::Data`] or [`Frame::Trailers`] frame.
    /// - `Poll::Ready(Some(Err(e)))` — a terminal error; no further frames will be produced.
    /// - `Poll::Ready(None)` — the body is fully consumed.
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Self::Data, Self::Error>>>;

    /// Indicates whether the body has been fully consumed.
    ///
    /// - `true` — the body is exhausted and [`Body::poll_frame`] should not be called again.
    /// - `false` — the body may still have frames to yield, or its state is unknown.
    ///
    /// Implementors SHOULD override this method when possible. Accurate reporting improves
    /// correctness and performance of consumers that can skip polling when the body is empty.
    #[inline]
    fn is_end_stream(&self) -> bool {
        false
    }

    /// Returns a hint about the total size of the remaining [`Frame::Data`] payload.
    ///
    /// The hint applies only to data frames; trailer frames are not included.
    ///
    /// Implementors SHOULD override this method when possible. An accurate size hint allows
    /// consumers to pre-allocate buffers and set content-length headers.
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

impl<B> Body for Pin<B>
where
    B: DerefMut,
    B::Target: Body,
{
    type Data = <B::Target as Body>::Data;
    type Error = <B::Target as Body>::Error;

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
