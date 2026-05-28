use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use pin_project_lite::pin_project;
use tokio::{
    sync::watch,
    time::{Instant, Sleep, sleep_until},
};

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
    type Output = Result<F::Output, KeepAliveOutput>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.fut.poll(cx) {
            Poll::Ready(res) => Poll::Ready(Ok(res)),
            Poll::Pending => this.timer.as_mut().poll(cx).map(Err),
        }
    }
}

pin_project! {
    /// A timer lazily reset the deadline after each successful poll(previous deadline met).
    ///
    /// This timer would optimistically assume deadline is not likely to be reached often.
    /// It has little cost inserting a new deadline and additional cost when previous
    /// deadline is met and the lazy reset happen with new deadline.
    pub struct KeepAlive {
        #[pin]
        timer: Sleep,
        deadline: Instant,
        shutdown: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    }
}

impl KeepAlive {
    #[inline]
    pub fn new(deadline: Instant, shutdown: Option<Shutdown>) -> Self {
        Self {
            timer: sleep_until(deadline),
            deadline,
            shutdown: shutdown.map(|s| {
                let mut receiver = s.receiver;

                Box::pin(async move {
                    let _ = receiver.changed().await;
                }) as Pin<Box<dyn Future<Output = ()> + Send>>
            }),
        }
    }

    #[cfg(any(feature = "http1", feature = "http2"))]
    #[inline]
    pub fn update(self: Pin<&mut Self>, deadline: Instant) {
        *self.project().deadline = deadline;
    }

    #[inline]
    pub fn reset(self: Pin<&mut Self>) {
        let this = self.project();
        this.timer.reset(*this.deadline)
    }

    fn is_expired(&self) -> bool {
        self.timer.deadline() >= self.deadline
    }
}

impl Future for KeepAlive {
    type Output = KeepAliveOutput;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        if let Some(shutdown_fut) = this.shutdown.as_mut() {
            if shutdown_fut.as_mut().poll(cx).is_ready() {
                return Poll::Ready(KeepAliveOutput::Cancel);
            }
        }

        ready!(this.timer.poll(cx));

        if self.is_expired() {
            Poll::Ready(KeepAliveOutput::Expire)
        } else {
            self.as_mut().reset();
            self.poll(cx)
        }
    }
}

/// return type of timer when it's finished
pub enum KeepAliveOutput {
    /// Timer is canceled by foreign input (e.g. a shutdown signal)
    Cancel,
    /// Timer is expired
    Expire,
}

/// A cloneable shutdown receiver. Pass one (or a clone) to [`KeepAlive::with_shutdown`]
/// for every connection that should observe the same shutdown signal.
#[derive(Clone)]
pub struct Shutdown {
    receiver: watch::Receiver<bool>,
}

impl Shutdown {
    /// Creates a new `Shutdown` token together with the [`ShutdownHandle`] that
    /// can trigger it. Clone the returned `Shutdown` to hand it to multiple
    /// [`KeepAlive`] instances.
    pub fn new() -> (Self, ShutdownHandle) {
        let (sender, receiver) = watch::channel(false);
        (Self { receiver }, ShutdownHandle { sender })
    }
}

/// Triggers shutdown across every [`KeepAlive`] instance that was created with
/// the associated [`Shutdown`] token.
///
/// Dropping this handle does **not** trigger shutdown; call [`shutdown`](ShutdownHandle::shutdown)
/// explicitly.
pub struct ShutdownHandle {
    sender: watch::Sender<bool>,
}

impl xitca_service::shutdown::Shutdown for ShutdownHandle {
    fn shutdown(&self) {
        // Ignore the error: it only occurs when all receivers have been dropped,
        // which means there is nothing left to notify.
        let _ = self.sender.send(true);
    }
}
