use core::{
    any::Any,
    future::Future,
    panic::{AssertUnwindSafe, UnwindSafe},
    pin::Pin,
    task::{Context, Poll},
};

use alloc::boxed::Box;

/// Future for the catch unwind async block.
pub struct CatchUnwind<Fut> {
    fut: Fut,
}

impl<Fut> CatchUnwind<Fut>
where
    Fut: Future + UnwindSafe,
{
    #[inline]
    pub const fn new(fut: Fut) -> Self {
        Self { fut }
    }
}

impl<Fut> Future for CatchUnwind<Fut>
where
    Fut: Future + UnwindSafe,
{
    type Output = Result<Fut::Output, Box<dyn Any + Send>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        std::panic::catch_unwind(AssertUnwindSafe(|| {
            // SAFETY:
            // fut is not moved.
            unsafe { self.map_unchecked_mut(|this| &mut this.fut) }.poll(cx)
        }))?
        .map(Ok)
    }
}
