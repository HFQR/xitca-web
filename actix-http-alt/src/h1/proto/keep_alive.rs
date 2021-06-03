use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use pin_project_lite::pin_project;
use tokio::time::{sleep_until, Instant, Sleep};

pin_project! {
    /// A keep alive timer lazily reset the deadline.
    /// after each successful poll.
    pub(super) struct KeepAlive {
        #[pin]
        timer: Sleep,
        dur: Duration,
        deadline: Instant,
    }
}

impl KeepAlive {
    // time is passed from outside of keep alive to reduce overhead
    // of timer syscall.
    pub(super) fn new(dur: Duration, now: Instant) -> Self {
        let deadline = now + dur;
        Self {
            timer: sleep_until(deadline),
            dur,
            deadline,
        }
    }

    #[inline(always)]
    pub(super) fn update(self: Pin<&mut Self>, now: Instant) {
        let this = self.project();
        *this.deadline = now + *this.dur;
    }

    pub(super) fn is_expired(&self) -> bool {
        self.timer.deadline() >= self.deadline
    }

    pub(super) fn reset(self: Pin<&mut Self>) {
        let this = self.project();
        this.timer.reset(*this.deadline)
    }
}

impl Future for KeepAlive {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().timer.poll(cx)
    }
}
