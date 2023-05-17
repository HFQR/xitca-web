use core::{
    fmt,
    future::{pending, Future},
    mem::{self, ManuallyDrop},
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

/// Copied from [ReusableBoxFuture](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.ReusableBoxFuture.html).
/// But without `Send` bound.
pub struct ReusableLocalBoxFuture<'a, T> {
    boxed: Pin<Box<dyn Future<Output = T> + 'a>>,
}

impl<'a, T> ReusableLocalBoxFuture<'a, T> {
    pub fn new<F>(future: F) -> Self
    where
        F: Future<Output = T> + 'a,
    {
        Self {
            boxed: Box::pin(future),
        }
    }

    pub fn set<F>(&mut self, future: F)
    where
        F: Future<Output = T> + 'a,
    {
        if let Err(future) = self.try_set(future) {
            *self = Self::new(future);
        }
    }

    fn try_set<F>(&mut self, future: F) -> Result<(), F>
    where
        F: Future<Output = T> + 'a,
    {
        // If we try to inline the contents of this function, the type checker complains because
        // the bound `T: 'a` is not satisfied in the call to `pending()`. But by putting it in an
        // inner function that doesn't have `T` as a generic parameter, we implicitly get the bound
        // `F::Output: 'a` transitively through `F: 'a`, allowing us to call `pending()`.
        #[inline(always)]
        fn real_try_set<'a, F>(this: &mut ReusableLocalBoxFuture<'a, F::Output>, future: F) -> Result<(), F>
        where
            F: Future + 'a,
        {
            // future::Pending<T> is a ZST so this never allocates.
            let boxed = mem::replace(&mut this.boxed, Box::pin(pending()));
            reuse_pin_box(boxed, future, |boxed| this.boxed = Pin::from(boxed))
        }

        real_try_set(self, future)
    }

    pub fn get_pin(&mut self) -> Pin<&mut dyn Future<Output = T>> {
        self.boxed.as_mut()
    }
}

impl<T> fmt::Debug for ReusableLocalBoxFuture<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReusableLocalBoxFuture").finish()
    }
}

fn reuse_pin_box<T: ?Sized, U, O, F>(boxed: Pin<Box<T>>, new_value: U, callback: F) -> Result<O, U>
where
    F: FnOnce(Box<U>) -> O,
{
    use std::alloc::Layout;

    let layout = Layout::for_value::<T>(&*boxed);
    if layout != Layout::new::<U>() {
        return Err(new_value);
    }

    // SAFETY: We don't ever construct a non-pinned reference to the old `T` from now on, and we
    // always drop the `T`.
    let raw: *mut T = Box::into_raw(unsafe { Pin::into_inner_unchecked(boxed) });

    // When dropping the old value panics, we still want to call `callback` — so move the rest of
    // the code into a guard type.
    let guard = CallOnDrop::new(|| {
        let raw: *mut U = raw.cast::<U>();
        unsafe { raw.write(new_value) };

        // SAFETY:
        // - `T` and `U` have the same layout.
        // - `raw` comes from a `Box` that uses the same allocator as this one.
        // - `raw` points to a valid instance of `U` (we just wrote it in).
        let boxed = unsafe { Box::from_raw(raw) };

        callback(boxed)
    });

    // Drop the old value.
    unsafe { ptr::drop_in_place(raw) };

    // Run the rest of the code.
    Ok(guard.call())
}

struct CallOnDrop<O, F: FnOnce() -> O> {
    f: ManuallyDrop<F>,
}

impl<O, F: FnOnce() -> O> CallOnDrop<O, F> {
    fn new(f: F) -> Self {
        let f = ManuallyDrop::new(f);
        Self { f }
    }
    fn call(self) -> O {
        let mut this = ManuallyDrop::new(self);
        let f = unsafe { ManuallyDrop::take(&mut this.f) };
        f()
    }
}

impl<O, F: FnOnce() -> O> Drop for CallOnDrop<O, F> {
    fn drop(&mut self) {
        let f = unsafe { ManuallyDrop::take(&mut self.f) };
        f();
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
