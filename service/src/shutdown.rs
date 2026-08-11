use alloc::{boxed::Box, vec::Vec};

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use spin::Mutex;

/// A thread-safe token that tracks whether a shutdown has been requested.
///
/// `ShutdownToken` is designed to be shared by reference (`&ShutdownToken`).
/// Calling [`ShutdownToken::shutdown`] marks the token as shut down and
/// immediately wakes all futures that are waiting on it.
pub struct ShutdownToken {
    inner: Mutex<ShutdownInner>,
}

struct ShutdownInner {
    is_shutdown: bool,
    wakers: Vec<Waker>,
}

impl ShutdownToken {
    /// Create a new `ShutdownToken` in the non-shutdown state.
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(ShutdownInner {
                is_shutdown: false,
                wakers: Vec::new(),
            }),
        }
    }

    /// Mark this token as shut down, waking all futures that are waiting on it.
    ///
    /// Any armed [`ShutdownFuture`] referencing this token will resolve to
    /// `None` as soon as its executor re-polls it.
    pub fn shutdown(&self) {
        let wakers = {
            let mut inner = self.inner.lock();
            inner.is_shutdown = true;
            // Take the wakers out so we can wake them outside the lock.
            core::mem::take(&mut inner.wakers)
        };
        for waker in wakers {
            waker.wake();
        }
    }

    /// Returns `true` if [`ShutdownToken::shutdown`] has been called.
    pub fn is_shutdown(&self) -> bool {
        self.inner.lock().is_shutdown
    }

    /// Register a waker to be notified when shutdown occurs.
    ///
    /// If shutdown has already happened the waker is woken immediately.
    fn register_waker(&self, waker: &Waker) {
        let mut inner = self.inner.lock();

        if inner.is_shutdown {
            drop(inner);
            waker.wake_by_ref();
            return;
        }

        // Avoid duplicating wakers for the same task.
        for existing in inner.wakers.iter() {
            if existing.will_wake(waker) {
                return;
            }
        }
        inner.wakers.push(waker.clone());
    }
}

impl Default for ShutdownToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension trait that adds a `.with_shutdown(token, armed)` combinator to any [`Future`].
pub trait ShutdownFutureExt: Future + Sized {
    /// Wrap this future so that it can be cancelled by a [`ShutdownToken`].
    ///
    /// The inner future is boxed so that it can be pinned without requiring
    /// `Unpin`.
    ///
    /// - When `armed` is `true` **and** the token is (or becomes) shut down, the
    ///   future resolves to `None` — the executor is woken immediately when
    ///   [`ShutdownToken::shutdown`] is called.
    /// - When `armed` is `false`, the shutdown token is ignored and the future
    ///   behaves as if it were unwrapped, resolving to `Some(output)`.
    ///
    /// This lets the caller decide at construction time whether to honour
    /// shutdown — for example, only opt in when a buffer is empty so that
    /// in-flight work is not interrupted mid-way.
    fn with_shutdown(self, token: &ShutdownToken, armed: bool) -> ShutdownFuture<'_, Self>;
}

impl<F: Future> ShutdownFutureExt for F {
    fn with_shutdown(self, token: &ShutdownToken, armed: bool) -> ShutdownFuture<'_, Self> {
        ShutdownFuture {
            future: Box::pin(self),
            token,
            armed,
        }
    }
}

/// A future that resolves to `Some(T)` if the inner future completes (or if
/// shutdown is not armed), or `None` if shutdown is armed and the token is
/// (or becomes) shut down.
pub struct ShutdownFuture<'a, F: Future> {
    future: Pin<Box<F>>,
    token: &'a ShutdownToken,
    armed: bool,
}

impl<F: Future> Future for ShutdownFuture<'_, F> {
    type Output = Option<F::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if !this.armed {
            return this.future.as_mut().poll(cx).map(Some);
        }

        if this.token.is_shutdown() {
            return Poll::Ready(None);
        }

        if let Poll::Ready(output) = this.future.as_mut().poll(cx) {
            return Poll::Ready(Some(output));
        }

        // Register the waker so we get notified when shutdown occurs.
        // register_waker atomically checks is_shutdown inside the lock,
        // so no separate double-check is needed.
        this.token.register_waker(cx.waker());

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_starts_not_shutdown() {
        let token = ShutdownToken::new();
        assert!(!token.is_shutdown());
    }

    #[test]
    fn token_shutdown_sets_state() {
        let token = ShutdownToken::new();
        token.shutdown();
        assert!(token.is_shutdown());
    }

    #[test]
    fn shared_ref_observes_shutdown() {
        let token = ShutdownToken::new();
        let r = &token;
        token.shutdown();
        assert!(r.is_shutdown());
    }

    #[test]
    fn armed_and_already_shutdown_resolves_none() {
        let token = ShutdownToken::new();
        token.shutdown();

        let fut = core::future::ready(42).with_shutdown(&token, true);
        let result = pollster_block_on(fut);
        assert_eq!(result, None);
    }

    #[test]
    fn not_armed_ignores_shutdown() {
        let token = ShutdownToken::new();
        token.shutdown();

        let fut = core::future::ready(42).with_shutdown(&token, false);
        let result = pollster_block_on(fut);
        assert_eq!(result, Some(42));
    }

    #[test]
    fn armed_future_completes_before_shutdown() {
        let token = ShutdownToken::new();
        let fut = core::future::ready(42).with_shutdown(&token, true);
        let result = pollster_block_on(fut);
        assert_eq!(result, Some(42));
    }

    #[test]
    fn shutdown_wakes_registered_waker() {
        use alloc::sync::Arc;
        use alloc::task::Wake;
        use core::sync::atomic::{AtomicUsize, Ordering};

        struct CountWake(AtomicUsize);
        impl Wake for CountWake {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let counter = Arc::new(CountWake(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());

        let token = ShutdownToken::new();
        token.register_waker(&waker);

        assert_eq!(counter.0.load(Ordering::SeqCst), 0);
        token.shutdown();
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn duplicate_waker_not_registered_twice() {
        use alloc::sync::Arc;
        use alloc::task::Wake;
        use core::sync::atomic::{AtomicUsize, Ordering};

        struct CountWake(AtomicUsize);
        impl Wake for CountWake {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let counter = Arc::new(CountWake(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());

        let token = ShutdownToken::new();
        token.register_waker(&waker);
        token.register_waker(&waker);

        token.shutdown();
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    }

    /// Minimal single-threaded block_on for tests (no tokio needed).
    fn pollster_block_on<F: Future>(fut: F) -> F::Output {
        let mut fut = core::pin::pin!(fut);
        let waker = core::task::Waker::noop();
        let mut cx = Context::from_waker(waker);

        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(val) => return val,
            Poll::Pending => {
                panic!("future returned Pending in a synchronous test");
            }
        }
    }
}
