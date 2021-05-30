use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Creates a future that wraps a function returning [`Poll`].
///
/// Polling the future delegates to the wrapped function.
///
/// # Examples
///
/// ```
/// #![feature(future_poll_fn)]
/// # async fn run() {
/// use core::future::poll_fn;
/// use std::task::{Context, Poll};
///
/// fn read_line(_cx: &mut Context<'_>) -> Poll<String> {
///     Poll::Ready("Hello, World!".into())
/// }
///
/// let read_future = poll_fn(read_line);
/// assert_eq!(read_future.await, "Hello, World!".to_owned());
/// # }
/// ```
pub fn poll_fn<T, F>(f: F) -> PollFn<F>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    PollFn { f }
}

/// A Future that wraps a function returning [`Poll`].
///
/// This `struct` is created by [`poll_fn()`]. See its
/// documentation for more.
#[must_use = "futures do nothing unless you `.await` or poll them"]
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

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        (&mut self.f)(cx)
    }
}
