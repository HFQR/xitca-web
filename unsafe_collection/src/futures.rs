use core::{
    fmt,
    future::Future,
    pin::Pin,
    ptr,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

/// Biased select always prioritize polling Self.
pub trait Select: Sized {
    fn select<Fut>(self, other: Fut) -> SelectFuture<Self, Fut>
    where
        Fut: Future;
}

impl<F> Select for F
where
    F: Future,
{
    #[inline]
    fn select<Fut>(self, other: Fut) -> SelectFuture<Self, Fut>
    where
        Fut: Future,
    {
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

/// Trait for trying to complete future in a single poll. And panic when fail to do so.
///
/// Useful for async code that only expose async API but not really doing stuff in async manner.
///
/// # Examples
/// ```rust
/// # use xitca_unsafe_collection::futures::NowOrPanic;
///
/// async fn looks_like() {
///     // nothing async really happened.
/// }
///
/// looks_like().now_or_panic();
/// ```
pub trait NowOrPanic: Sized {
    type Output;

    fn now_or_panic(&mut self) -> Self::Output;
}

impl<F> NowOrPanic for F
where
    F: Future,
{
    type Output = F::Output;

    fn now_or_panic(&mut self) -> Self::Output {
        let waker = noop_waker();
        let cx = &mut Context::from_waker(&waker);

        // SAFETY:
        // self is not moved.
        match unsafe { Pin::new_unchecked(self).poll(cx) } {
            Poll::Ready(ret) => ret,
            Poll::Pending => panic!("Future can not be polled to complete"),
        }
    }
}

const TBL: RawWakerVTable = RawWakerVTable::new(|_| raw_waker(), |_| {}, |_| {}, |_| {});

const fn raw_waker() -> RawWaker {
    RawWaker::new(ptr::null(), &TBL)
}

pub(crate) fn noop_waker() -> Waker {
    // SAFETY:
    // no op waker uphold all the rules of RawWaker and RawWakerVTable
    unsafe { Waker::from_raw(raw_waker()) }
}

#[cfg(test)]
mod test {
    use core::{future::poll_fn, pin::pin};

    use super::*;

    #[test]
    fn test_select() {
        let fut = async {
            poll_fn(|cx| {
                cx.waker().wake_by_ref();
                Poll::<()>::Pending
            })
            .await;
            123
        }
        .select(async { 321 });

        matches!(pin!(fut).now_or_panic(), SelectOutput::B(321));
    }
}
