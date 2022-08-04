use std::{
    cell::RefCell,
    collections::VecDeque,
    future::poll_fn,
    future::Future,
    io,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

use futures_core::Stream;

use crate::{bytes::Bytes, error::BodyError};

/// max buffer size 32k
pub(crate) const MAX_BUFFER_SIZE: usize = 32_768;

/// Buffered stream of bytes chunks
///
/// Payload stores chunks in a vector. First chunk can be received with
/// `.poll_read()` method. Payload stream is not thread safe. Payload does not
/// notify current task when new data is available.
///
/// Payload stream can be used as `Response` body stream.
#[derive(Debug)]
pub struct RequestBody(Rc<RefCell<Inner>>);

impl Default for RequestBody {
    fn default() -> Self {
        Self::create(true)
    }
}

impl RequestBody {
    /// Create request body stream with given EOF state.
    pub(super) fn create(eof: bool) -> Self {
        RequestBody(Rc::new(RefCell::new(Inner::new(eof))))
    }

    /// Create RequestBodySender together with RequestBody that share the same inner body state.
    /// RequestBodySender is used to mutate data/eof/error state and made the change observable
    /// from RequestBody owner.
    pub(super) fn channel(eof: bool) -> (RequestBodySender, Self) {
        let this = Self::create(eof);
        (RequestBodySender(this.0.clone()), this)
    }
}

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, BodyError>>> {
        self.0.borrow_mut().poll_read(cx)
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H1(body)
    }
}

/// Sender part of the payload stream
pub struct RequestBodySender(Rc<RefCell<Inner>>);

// TODO: rework early eof error handling.
impl Drop for RequestBodySender {
    fn drop(&mut self) {
        if self.payload_alive() {
            let mut inner = self.0.borrow_mut();
            if !inner.eof {
                inner.feed_error(BodyError::Io(io::ErrorKind::UnexpectedEof.into()));
                inner.wake();
            }
        }
    }
}

impl RequestBodySender {
    pub(super) fn is_eof(&self) -> bool {
        self.0.borrow_mut().eof
    }

    pub(super) fn feed_error(&mut self, err: BodyError) {
        if self.payload_alive() {
            self.0.borrow_mut().feed_error(err);
        }
    }

    pub(super) fn feed_eof(&mut self) {
        if self.payload_alive() {
            self.0.borrow_mut().feed_eof();
        }
    }

    // TODO: feed_data should return the payload_alive status.
    pub(super) fn feed_data(&mut self, data: Bytes) {
        if self.payload_alive() {
            self.0.borrow_mut().feed_data(data);
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
            if self.payload_alive() {
                let mut borrow = self.0.borrow_mut();
                if func(&mut *borrow) {
                    Poll::Ready(Ok(()))
                } else {
                    // when payload is not ready register current task waker and wait.
                    borrow.register_io(cx);
                    Poll::Pending
                }
            } else {
                Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()))
            }
        })
        .await
    }

    /// Payload share the same `Rc` with Sender.
    /// Strong count 1 means payload is already dropped.
    fn payload_alive(&self) -> bool {
        debug_assert!(Rc::strong_count(&self.0) <= 2);
        debug_assert_eq!(Rc::weak_count(&self.0), 0);
        Rc::strong_count(&self.0) != 1
    }
}

#[derive(Debug)]
struct Inner {
    len: usize,
    eof: bool,
    err: Option<BodyError>,
    items: VecDeque<Bytes>,
    task: Option<Waker>,
    io_task: Option<Waker>,
}

impl Inner {
    fn new(eof: bool) -> Self {
        Inner {
            eof,
            len: 0,
            err: None,
            items: VecDeque::new(),
            task: None,
            io_task: None,
        }
    }

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
    fn register(&mut self, cx: &mut Context<'_>) {
        if self.task.as_ref().map(|w| !cx.waker().will_wake(w)).unwrap_or(true) {
            self.task = Some(cx.waker().clone());
        }
    }

    // Register future feeding data to payload.
    /// Waker would be used in `Inner::wake_io`
    fn register_io(&mut self, cx: &mut Context<'_>) {
        if self.io_task.as_ref().map(|w| !cx.waker().will_wake(w)).unwrap_or(true) {
            self.io_task = Some(cx.waker().clone());
        }
    }

    fn feed_error(&mut self, err: BodyError) {
        self.err = Some(err);
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

    fn poll_read(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, BodyError>>> {
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
