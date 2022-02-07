use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use tokio::time::Sleep;

pub(crate) async fn never<T>() -> T {
    poll_fn(|_| Poll::Pending).await
}

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

impl<F> fmt::Debug for PollFn<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollFn").finish()
    }
}

impl<T, F> Future for PollFn<F>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        (self.f)(cx)
    }
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
    fn timeout(self, timer: Pin<&mut Sleep>) -> TimeoutFuture<'_, Self>;
}

impl<F> Timeout for F
where
    F: Future,
{
    fn timeout(self, timer: Pin<&mut Sleep>) -> TimeoutFuture<'_, Self> {
        TimeoutFuture { fut: self, timer }
    }
}

pin_project! {
    pub(crate) struct TimeoutFuture<'a, F> {
        #[pin]
        fut: F,
        timer: Pin<&'a mut Sleep>
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
