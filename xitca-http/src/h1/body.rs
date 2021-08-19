use std::{
    cell::RefCell,
    collections::VecDeque,
    io,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

use bytes::Bytes;
use futures_core::Stream;

use crate::error::BodyError;
use crate::util::futures::poll_fn;

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

impl RequestBody {
    /// Create payload stream.
    ///
    /// This method construct two objects responsible for bytes stream
    /// generation.
    ///
    /// * `PayloadSender` - *Sender* side of the stream
    ///
    /// * `Payload` - *Receiver* side of the stream
    pub(super) fn create(eof: bool) -> (RequestBodySender, Self) {
        let shared = Rc::new(RefCell::new(Inner::new(eof)));

        (RequestBodySender(shared.clone()), Self(shared))
    }

    /// Create empty payload
    pub(super) fn empty() -> Self {
        Self(Rc::new(RefCell::new(Inner::new(true))))
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
        debug_assert!(self.payload_alive());
        self.0.borrow_mut().feed_eof();
    }

    pub(super) fn feed_data(&mut self, data: Bytes) {
        debug_assert!(self.payload_alive());
        self.0.borrow_mut().feed_data(data);
    }

    pub(super) async fn ready(&self) -> io::Result<()> {
        poll_fn(|cx| self.poll_ready(cx)).await
    }

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // we check backpressure only if Payload (other side) is alive,
        // otherwise always return io error.
        if self.payload_alive() {
            let mut borrow = self.0.borrow_mut();
            // when payload is backpressure register current task waker and wait.
            if borrow.backpressure() {
                borrow.register_io(cx);
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        } else {
            Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof)))
        }
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
