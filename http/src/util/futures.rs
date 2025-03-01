use pin_project_lite::pin_project;
#[cfg(any(feature = "http2", feature = "http3"))]
pub(crate) use queue::*;
use std::future::Future;

#[cfg(any(feature = "http2", feature = "http3"))]
mod queue {
    use futures_util::stream::{FuturesUnordered, StreamExt};

    pub(crate) struct Queue<F>(FuturesUnordered<F>);

    impl<F: Future> Queue<F> {
        pub(crate) fn new() -> Self {
            Self(FuturesUnordered::new())
        }

        #[cfg(any(all(feature = "http2", feature = "io-uring"), feature = "http3"))]
        pub(crate) async fn next(&mut self) -> F::Output {
            if self.is_empty() {
                core::future::pending().await
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

// A future that resolve only one time when the future is ready
pin_project! {
    pub(crate) struct WaitOrPending<F> {
        #[pin]
        future: F,
        is_pending: bool,
    }
}

impl<F> WaitOrPending<F> {
    pub fn new(future: F, is_pending: bool) -> Self {
        Self { future, is_pending }
    }
}

impl<F: Future> Future for WaitOrPending<F> {
    type Output = F::Output;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        if self.is_pending {
            return std::task::Poll::Pending;
        }

        let this = self.as_mut().project();

        match this.future.poll(cx) {
            std::task::Poll::Ready(f) => {
                *this.is_pending = true;

                std::task::Poll::Ready(f)
            }
            poll => poll,
        }
    }
}
