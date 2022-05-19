use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

#[inline]
pub async fn never<T>() -> T {
    poll_fn(|_| Poll::Pending).await
}

#[inline]
pub fn poll_fn<T, F>(f: F) -> PollFn<F>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    PollFn { f }
}

pub struct PollFn<F> {
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

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        (self.f)(cx)
    }
}

pub trait Select: Sized {
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

pub struct SelectFuture<Fut1, Fut2> {
    fut1: Fut1,
    fut2: Fut2,
}

impl<Fut1, Fut2> Future for SelectFuture<Fut1, Fut2>
where
    Fut1: Future,
    Fut2: Future,
{
    type Output = SelectOutput<Fut1::Output, Fut2::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY:
        // This is safe as Fut1 and Fut2 do not move.
        // They both accessed only through a single Pin<&mut _>.
        unsafe {
            let Self { fut1, fut2 } = self.get_unchecked_mut();

            if let Poll::Ready(a) = Pin::new_unchecked(fut1).poll(cx) {
                return Poll::Ready(SelectOutput::A(a));
            }

            Pin::new_unchecked(fut2).poll(cx).map(SelectOutput::B)
        }
    }
}

pub enum SelectOutput<A, B> {
    A(A),
    B(B),
}

impl<A, B> fmt::Debug for SelectOutput<A, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::A(_) => f.debug_struct("SelectOutput::A(..)"),
            Self::B(_) => f.debug_struct("SelectOutput::B(..)"),
        }
        .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    extern crate alloc;

    use core::task::Waker;

    use alloc::{sync::Arc, task::Wake};

    struct DummyWaker;

    impl Wake for DummyWaker {
        fn wake(self: Arc<Self>) {
            // do nothing.
        }
    }

    #[test]
    fn test_select() {
        let mut fut = async {
            poll_fn(|cx| {
                cx.waker().wake_by_ref();
                Poll::<()>::Pending
            })
            .await;
            123
        }
        .select(async { 321 });

        let fut = unsafe { Pin::new_unchecked(&mut fut) };

        let waker = Waker::from(Arc::new(DummyWaker));

        let cx = &mut Context::from_waker(&waker);

        matches!(fut.poll(cx), Poll::Ready(SelectOutput::B(321)));
    }
}
