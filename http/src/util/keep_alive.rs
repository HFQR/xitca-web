use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::ready;
use pin_project_lite::pin_project;
use tokio::time::{sleep_until, Instant, Sleep};

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

pub struct KeepAliveExpired;

impl Future for KeepAlive {
    type Output = KeepAliveExpired;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();
        ready!(this.timer.poll(cx));

        if self.is_expired() {
            Poll::Ready(KeepAliveExpired)
        } else {
            self.as_mut().reset();
            self.poll(cx)
        }
    }
}
