use std::{
    cell::RefCell,
    collections::VecDeque,
    io,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context, Poll, Waker},
};

use bytes::Bytes;
use futures_core::Stream;

use crate::error::BodyError;

/// max buffer size 32k
pub(crate) const MAX_BUFFER_SIZE: usize = 32_768;

/// Buffered stream of bytes chunks
///
/// Payload stores chunks in a vector. First chunk can be received with
/// `.readany()` method. Payload stream is not thread safe. Payload does not
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
    pub fn create(eof: bool) -> (RequestBodySender, Self) {
        let shared = Rc::new(RefCell::new(Inner::new(eof)));

        (RequestBodySender(Rc::downgrade(&shared)), Self(shared))
    }

    #[doc(hidden)]
    /// Create empty payload
    pub fn empty() -> Self {
        Self(Rc::new(RefCell::new(Inner::new(true))))
    }

    /// Length of the data in this payload
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    /// Is payload empty
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn readany(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, BodyError>>> {
        self.0.borrow_mut().readany(cx)
    }
}

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, BodyError>>> {
        self.0.borrow_mut().readany(cx)
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H1(body)
    }
}

/// Sender part of the payload stream
pub struct RequestBodySender(Weak<RefCell<Inner>>);

impl RequestBodySender {
    #[inline]
    pub fn set_error(&mut self, err: BodyError) {
        if let Some(shared) = self.0.upgrade() {
            shared.borrow_mut().set_error(err)
        }
    }

    #[inline]
    pub fn feed_eof(&mut self) {
        if let Some(shared) = self.0.upgrade() {
            shared.borrow_mut().feed_eof()
        }
    }

    #[inline]
    pub fn feed_data(&mut self, data: Bytes) {
        if let Some(shared) = self.0.upgrade() {
            shared.borrow_mut().feed_data(data)
        }
    }

    #[inline]
    pub fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // we check need_read only if Payload (other side) is alive,
        // otherwise always return io error.
        self.0
            .upgrade()
            .map(|shared| {
                let mut borrow = shared.borrow_mut();
                if borrow.need_read {
                    Poll::Ready(Ok(()))
                } else {
                    borrow.register_io(cx);
                    Poll::Pending
                }
            })
            .unwrap_or(Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof))))
    }
}

#[derive(Debug)]
struct Inner {
    len: usize,
    eof: bool,
    err: Option<BodyError>,
    need_read: bool,
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
            need_read: true,
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

    #[inline]
    fn set_error(&mut self, err: BodyError) {
        self.err = Some(err);
    }

    #[inline]
    fn feed_eof(&mut self) {
        self.eof = true;
    }

    #[inline]
    fn feed_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_back(data);
        self.need_read = self.len < MAX_BUFFER_SIZE;
        self.wake();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.len
    }

    fn readany(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, BodyError>>> {
        if let Some(data) = self.items.pop_front() {
            self.len -= data.len();
            self.need_read = self.len < MAX_BUFFER_SIZE;

            if self.need_read && !self.eof {
                self.register(cx);
            }
            self.wake_io();
            Poll::Ready(Some(Ok(data)))
        } else if let Some(err) = self.err.take() {
            Poll::Ready(Some(Err(err)))
        } else if self.eof {
            Poll::Ready(None)
        } else {
            self.need_read = true;
            self.register(cx);
            self.wake_io();
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::util::poll_fn::poll_fn;

    #[tokio::test]
    async fn test_unread_data() {
        let (_, mut payload) = RequestBody::create(false);

        payload.unread_data(Bytes::from("data"));
        assert!(!payload.is_empty());
        assert_eq!(payload.len(), 4);

        assert_eq!(
            Bytes::from("data"),
            poll_fn(|cx| payload.readany(cx)).await.unwrap().unwrap()
        );
    }
}
