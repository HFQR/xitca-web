use core::{
    cell::{RefCell, RefMut},
    future::{Future, poll_fn},
    ops::DerefMut,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use std::{collections::VecDeque, io, rc::Rc};

use futures_core::stream::Stream;

use crate::bytes::Bytes;

/// max buffer size 32k
pub(crate) const MAX_BUFFER_SIZE: usize = 32_768;

#[derive(Clone, Debug)]
enum RequestBodyInner {
    Some(Rc<RefCell<Inner>>),
    #[cfg(feature = "io-uring")]
    Completion(super::dispatcher_uring::Body),
    None,
}

impl RequestBodyInner {
    fn new(eof: bool) -> Self {
        match eof {
            true => Self::None,
            false => Self::Some(Default::default()),
        }
    }
}

/// Buffered stream of request body chunk.
///
/// impl [Stream] trait to produce chunk as [Bytes] type in async manner.
#[derive(Debug)]
pub struct RequestBody(RequestBodyInner);

impl Default for RequestBody {
    fn default() -> Self {
        Self(RequestBodyInner::new(true))
    }
}

impl RequestBody {
    // an async spsc channel where RequestBodySender used to push data and popped from RequestBody.
    pub(super) fn channel(eof: bool) -> (RequestBodySender, Self) {
        let inner = RequestBodyInner::new(eof);
        (RequestBodySender(inner.clone()), RequestBody(inner))
    }

    #[cfg(feature = "io-uring")]
    pub(super) fn io_uring(body: super::dispatcher_uring::Body) -> Self {
        RequestBody(RequestBodyInner::Completion(body))
    }
}

impl Stream for RequestBody {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<io::Result<Bytes>>> {
        match self.get_mut().0 {
            RequestBodyInner::Some(ref mut inner) => inner.borrow_mut().poll_next_unpin(cx),
            RequestBodyInner::None => Poll::Ready(None),
            #[cfg(feature = "io-uring")]
            RequestBodyInner::Completion(ref mut body) => Pin::new(body).poll_next(cx),
        }
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H1(body)
    }
}

/// Sender part of the payload stream
pub struct RequestBodySender(RequestBodyInner);

// TODO: rework early eof error handling.
impl Drop for RequestBodySender {
    fn drop(&mut self) {
        if let Some(mut inner) = self.try_inner() {
            if !inner.eof {
                inner.feed_error(io::ErrorKind::UnexpectedEof.into());
            }
        }
    }
}

impl RequestBodySender {
    // try to get a mutable reference of inner and ignore RequestBody::None variant.
    fn try_inner(&mut self) -> Option<RefMut<'_, Inner>> {
        self.try_inner_on_none_with(|| {})
    }

    // try to get a mutable reference of inner and panic on RequestBody::None variant.
    // this is a runtime check for internal optimization to avoid unnecessary operations.
    // public api must not be able to trigger this panic.
    fn try_inner_infallible(&mut self) -> Option<RefMut<'_, Inner>> {
        self.try_inner_on_none_with(|| panic!("No Request Body found. Do not waste operation on Sender."))
    }

    fn try_inner_on_none_with<F>(&mut self, func: F) -> Option<RefMut<'_, Inner>>
    where
        F: FnOnce(),
    {
        match self.0 {
            RequestBodyInner::Some(ref inner) => {
                // request body is a shared pointer between only two owners and no weak reference.
                debug_assert!(Rc::strong_count(inner) <= 2);
                debug_assert_eq!(Rc::weak_count(inner), 0);
                (Rc::strong_count(inner) != 1).then_some(inner.borrow_mut())
            }
            _ => {
                func();
                None
            }
        }
    }

    pub(super) fn feed_error(&mut self, e: io::Error) {
        if let Some(mut inner) = self.try_inner_infallible() {
            inner.feed_error(e);
        }
    }

    pub(super) fn feed_eof(&mut self) {
        if let Some(mut inner) = self.try_inner_infallible() {
            inner.feed_eof();
        }
    }

    pub(super) fn feed_data(&mut self, data: Bytes) {
        if let Some(mut inner) = self.try_inner_infallible() {
            inner.feed_data(data);
        }
    }

    pub(super) fn ready(&mut self) -> impl Future<Output = io::Result<()>> + '_ {
        self.ready_with(|inner| !inner.backpressure())
    }

    // Lazily wait until RequestBody is already polled.
    // For specific use case body must not be eagerly polled.
    // For example: Request with Expect: Continue header.
    pub(super) fn wait_for_poll(&mut self) -> impl Future<Output = io::Result<()>> + '_ {
        self.ready_with(|inner| inner.waiting())
    }

    async fn ready_with<F>(&mut self, func: F) -> io::Result<()>
    where
        F: Fn(&mut Inner) -> bool,
    {
        poll_fn(|cx| {
            // Check only if Payload (other side) is alive, Otherwise always return io error.
            match self.try_inner_infallible() {
                Some(mut inner) => {
                    if func(inner.deref_mut()) {
                        Poll::Ready(Ok(()))
                    } else {
                        // when payload is not ready register current task waker and wait.
                        inner.register_io(cx);
                        Poll::Pending
                    }
                }
                None => Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
            }
        })
        .await
    }
}

#[derive(Debug, Default)]
struct Inner {
    eof: bool,
    len: usize,
    err: Option<io::Error>,
    items: VecDeque<Bytes>,
    task: Option<Waker>,
    io_task: Option<Waker>,
}

impl Inner {
    /// Wake up future waiting for payload data to be available.
    fn wake(&mut self) {
        if let Some(waker) = self.task.take() {
            waker.wake();
        }
    }

    /// Wake up future feeding data to Payload.
    fn wake_io(&mut self) {
        if let Some(waker) = self.io_task.take() {
            waker.wake();
        }
    }

    /// true when a future is waiting for payload data.
    fn waiting(&self) -> bool {
        self.task.is_some()
    }

    /// Register future waiting data from payload.
    /// Waker would be used in `Inner::wake`
    fn register(&mut self, cx: &Context<'_>) {
        if self.task.as_ref().map(|w| !cx.waker().will_wake(w)).unwrap_or(true) {
            self.task = Some(cx.waker().clone());
        }
    }

    // Register future feeding data to payload.
    /// Waker would be used in `Inner::wake_io`
    fn register_io(&mut self, cx: &Context<'_>) {
        if self.io_task.as_ref().map(|w| !cx.waker().will_wake(w)).unwrap_or(true) {
            self.io_task = Some(cx.waker().clone());
        }
    }

    fn feed_error(&mut self, err: io::Error) {
        self.err = Some(err);
        self.wake();
    }

    fn feed_eof(&mut self) {
        self.eof = true;
        self.wake();
    }

    fn feed_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_back(data);
        self.wake();
    }

    fn backpressure(&self) -> bool {
        self.len >= MAX_BUFFER_SIZE
    }

    fn poll_next_unpin(&mut self, cx: &Context<'_>) -> Poll<Option<io::Result<Bytes>>> {
        if let Some(data) = self.items.pop_front() {
            self.len -= data.len();
            Poll::Ready(Some(Ok(data)))
        } else if let Some(err) = self.err.take() {
            Poll::Ready(Some(Err(err)))
        } else if self.eof {
            Poll::Ready(None)
        } else {
            self.register(cx);
            self.wake_io();
            Poll::Pending
        }
    }
}
