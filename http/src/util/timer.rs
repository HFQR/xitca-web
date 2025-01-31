use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, ready},
};

use pin_project_lite::pin_project;
use tokio::time::{Instant, Sleep, sleep_until};

pub(crate) trait Timeout: Sized {
    fn timeout(self, timer: Pin<&mut KeepAlive>) -> TimeoutFuture<'_, Self>;
}

impl<F> Timeout for F
where
    F: Future,
{
    fn timeout(self, timer: Pin<&mut KeepAlive>) -> TimeoutFuture<'_, Self> {
        TimeoutFuture { fut: self, timer }
    }
}

pin_project! {
    pub(crate) struct TimeoutFuture<'a, F> {
        #[pin]
        fut: F,
        timer: Pin<&'a mut KeepAlive>
    }
}

impl<F: Future> Future for TimeoutFuture<'_, F> {
    type Output = Result<F::Output, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.fut.poll(cx) {
            Poll::Ready(res) => Poll::Ready(Ok(res)),
            Poll::Pending => this.timer.as_mut().poll(cx).map(Err),
        }
    }
}

pin_project! {
    /// A timer lazily reset the deadline after each successful poll(previous deadline met).
    ///
    /// This timer would optimistically assume deadline is not likely to be reached often.
    /// It has little cost inserting a new deadline and additional cost when previous
    /// deadline is met and the lazy reset happen with new deadline.
    pub struct KeepAlive {
        #[pin]
        timer: Sleep,
        deadline: Instant,
    }
}

impl KeepAlive {
    #[inline]
    pub fn new(deadline: Instant) -> Self {
        Self {
            timer: sleep_until(deadline),
            deadline,
        }
    }

    #[cfg(any(feature = "http1", feature = "http2"))]
    #[inline]
    pub fn update(self: Pin<&mut Self>, deadline: Instant) {
        *self.project().deadline = deadline;
    }

    #[inline]
    pub fn reset(self: Pin<&mut Self>) {
        let this = self.project();
        this.timer.reset(*this.deadline)
    }

    fn is_expired(&self) -> bool {
        self.timer.deadline() >= self.deadline
    }
}

impl Future for KeepAlive {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();
        ready!(this.timer.poll(cx));

        if self.is_expired() {
            Poll::Ready(())
        } else {
            self.as_mut().reset();
            self.poll(cx)
        }
    }
}
