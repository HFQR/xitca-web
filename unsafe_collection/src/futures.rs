extern crate alloc;

use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use alloc::{sync::Arc, task::Wake};

#[macro_export]
macro_rules! pin {
    ($($x:ident),* $(,)?) => { $(
        // Move the value to ensure that it is owned
        let mut $x = $x;
        // Shadow the original binding so that it can't be directly accessed
        // ever again.
        #[allow(unused_mut)]
        let mut $x = unsafe {
            core::pin::Pin::new_unchecked(&mut $x)
        };
    )* }
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
        let waker = Arc::new(NoOpWaker).into();
        let cx = &mut Context::from_waker(&waker);

        // SAFETY:
        // self is not moved.
        match unsafe { Pin::new_unchecked(self).poll(cx) } {
            Poll::Ready(ret) => ret,
            Poll::Pending => panic!("Future can not be polled to complete"),
        }
    }
}

/// A waker that do nothing when [Waker::wake] and [Waker::wake_by_ref] is called.
pub struct NoOpWaker;

impl NoOpWaker {
    /// Construct a noop [Waker]
    pub fn waker() -> Waker {
        Waker::from(Arc::new(Self))
    }
}

impl Wake for NoOpWaker {
    fn wake(self: Arc<Self>) {
        // do nothing.
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use core::future::poll_fn;

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

        pin!(fut);

        matches!(fut.now_or_panic(), SelectOutput::B(321));
    }
}
