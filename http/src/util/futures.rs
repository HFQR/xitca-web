use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use super::keep_alive::KeepAlive;

#[inline]
pub(crate) fn poll_fn<T, F>(f: F) -> PollFn<F>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    PollFn { f }
}

pub(crate) struct PollFn<F> {
    f: F,
}

impl<F> Unpin for PollFn<F> {}

impl<T, F> Future for PollFn<F>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        (&mut self.f)(cx)
    }
}

/// An async function that never resolve to the output.
#[inline]
pub(crate) async fn never<T>() -> T {
    poll_fn(|_| Poll::Pending).await
}

pub(crate) trait Select: Sized {
    fn select<Fut>(self, other: Fut) -> SelectFuture<Self, Fut>;
}

impl<F> Select for F
where
    F: Future,
{
    #[inline]
    fn select<Fut>(self, other: Fut) -> SelectFuture<Self, Fut> {
        SelectFuture {
            fut1: self,
            fut2: other,
        }
    }
}

pin_project! {
    pub(crate) struct SelectFuture<Fut1, Fut2> {
        #[pin]
        fut1: Fut1,
        #[pin]
        fut2: Fut2,
    }
}

impl<Fut1, Fut2> Future for SelectFuture<Fut1, Fut2>
where
    Fut1: Future,
    Fut2: Future,
{
    type Output = SelectOutput<Fut1::Output, Fut2::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if let Poll::Ready(a) = this.fut1.poll(cx) {
            return Poll::Ready(SelectOutput::A(a));
        }

        this.fut2.poll(cx).map(SelectOutput::B)
    }
}

pub(crate) enum SelectOutput<A, B> {
    A(A),
    B(B),
}

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

        if let Poll::Ready(res) = this.fut.poll(cx) {
            return Poll::Ready(Ok(res));
        }

        this.timer.as_mut().poll(cx).map(Err)
    }
}

#[cfg(any(feature = "http2", feature = "http3"))]
pub(crate) use queue::*;

#[cfg(any(feature = "http2", feature = "http3"))]
mod queue {
    use super::*;

    use futures_util::stream::{FuturesUnordered, StreamExt};

    pub(crate) struct Queue<F> {
        queued: bool,
        futures: FuturesUnordered<F>,
    }

    impl<F: Future> Queue<F> {
        pub(crate) fn new() -> Self {
            Self {
                queued: false,
                futures: FuturesUnordered::new(),
            }
        }

        pub(crate) async fn next(&mut self) -> F::Output {
            if self.queued {
                match self.futures.next().await {
                    Some(res) => return res,
                    None => self.queued = false,
                }
            }

            never().await
        }

        pub(crate) fn push(&mut self, future: F) {
            self.futures.push(future);
            self.queued = true;
        }

        pub(crate) async fn drain(&mut self) {
            while self.futures.next().await.is_some() {}
        }
    }
}
