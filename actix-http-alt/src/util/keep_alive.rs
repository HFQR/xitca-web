use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::ready;
use pin_project::pin_project;
use tokio::time::{sleep_until, Instant, Sleep};

/// A keep alive timer lazily reset the deadline.
/// after each successful poll.
#[pin_project]
pub(crate) struct KeepAlive {
    #[pin]
    timer: Sleep,
    deadline: Instant,
}

impl KeepAlive {
    // time is passed from outside of keep alive to reduce overhead
    // of timer syscall.
    pub(crate) fn new(deadline: Instant) -> Self {
        Self {
            timer: sleep_until(deadline),
            deadline,
        }
    }

    #[inline(always)]
    pub(crate) fn update(self: Pin<&mut Self>, deadline: Instant) {
        let this = self.project();
        *this.deadline = deadline;
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.timer.deadline() >= self.deadline
    }

    pub(crate) fn reset(self: Pin<&mut Self>) {
        let this = self.project();
        this.timer.reset(*this.deadline)
    }
}

impl Future for KeepAlive {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();
        ready!(this.timer.poll(cx));

        if self.as_mut().is_expired() {
            Poll::Ready(())
        } else {
            self.as_mut().reset();
            self.poll(cx)
        }
    }
}
