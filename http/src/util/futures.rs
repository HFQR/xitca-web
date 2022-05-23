use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use super::keep_alive::{KeepAlive, KeepAliveExpired};

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
    type Output = Result<F::Output, KeepAliveExpired>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.fut.poll(cx) {
            Poll::Ready(res) => Poll::Ready(Ok(res)),
            Poll::Pending => this.timer.as_mut().poll(cx).map(Err),
        }
    }
}

#[cfg(any(feature = "http2", feature = "http3"))]
pub(crate) use queue::*;

#[cfg(any(feature = "http2", feature = "http3"))]
mod queue {
    use super::*;

    use futures_util::stream::{FuturesUnordered, StreamExt};

    pub(crate) struct Queue<F>(FuturesUnordered<F>);

    impl<F: Future> Queue<F> {
        pub(crate) fn new() -> Self {
            Self(FuturesUnordered::new())
        }

        #[cfg(feature = "http3")]
        pub(crate) async fn next(&mut self) -> F::Output {
            if self.is_empty() {
                xitca_unsafe_collection::futures::never().await
            } else {
                self.next2().await
            }
        }

        pub(crate) fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        pub(crate) async fn next2(&mut self) -> F::Output {
            self.0
                .next()
                .await
                .expect("Queue::next2 must be called when queue is not empty")
        }

        pub(crate) fn push(&self, future: F) {
            self.0.push(future);
        }

        pub(crate) async fn drain(&mut self) {
            while self.0.next().await.is_some() {}
        }
    }
}
