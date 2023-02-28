#[cfg(any(feature = "http2", feature = "http3"))]
pub(crate) use queue::*;

#[cfg(any(feature = "http2", feature = "http3"))]
mod queue {
    use std::future::Future;

    use futures_util::stream::{FuturesUnordered, StreamExt};

    pub(crate) struct Queue<F>(FuturesUnordered<F>);

    impl<F: Future> Queue<F> {
        pub(crate) fn new() -> Self {
            Self(FuturesUnordered::new())
        }

        #[cfg(any(feature = "http2", feature = "http3"))]
        pub(crate) async fn next(&mut self) -> F::Output {
            if self.is_empty() {
                std::future::pending().await
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
