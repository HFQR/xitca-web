use core::{
    cell::RefCell,
    future::poll_fn,
    task::{Poll, Waker},
};

use std::{io, rc::Rc};

use xitca_io::io::{AsyncBufRead, AsyncBufWrite, BoundedBuf};

use crate::bytes::BytesMut;

pub(super) trait BufIo {
    fn read(self, io: &impl AsyncBufRead) -> impl Future<Output = (io::Result<usize>, Self)>;

    fn write(self, io: &impl AsyncBufWrite) -> impl Future<Output = (io::Result<()>, Self)>;
}

impl BufIo for BytesMut {
    async fn read(mut self, io: &impl AsyncBufRead) -> (io::Result<usize>, Self) {
        let len = self.len();

        self.reserve(4096);

        let (res, buf) = io.read(self.slice(len..)).await;
        (res, buf.into_inner())
    }

    async fn write(self, io: &impl AsyncBufWrite) -> (io::Result<()>, Self) {
        let (res, mut buf) = io.write_all(self).await;
        buf.clear();
        (res, buf)
    }
}

pub(super) struct SharedIo<Io> {
    inner: Rc<_SharedIo<Io>>,
}

struct _SharedIo<Io> {
    io: Io,
    notify: RefCell<Inner<BytesMut>>,
}

impl<Io> SharedIo<Io> {
    pub(super) fn new(io: Io) -> Self {
        Self {
            inner: Rc::new(_SharedIo {
                io,
                notify: RefCell::new(Inner { waker: None, val: None }),
            }),
        }
    }

    #[inline(always)]
    pub(super) fn io(&self) -> &Io {
        &self.inner.io
    }

    pub(super) fn into_io(self) -> Io {
        Rc::try_unwrap(self.inner)
            .ok()
            .expect("SharedIo still has outstanding references")
            .io
    }

    pub(super) fn notifier(&mut self) -> NotifierIo<Io> {
        NotifierIo {
            inner: self.inner.clone(),
        }
    }

    pub(super) fn wait(&mut self) -> impl Future<Output = Option<BytesMut>> {
        poll_fn(|cx| {
            let mut inner = self.inner.notify.borrow_mut();
            if let Some(val) = inner.val.take() {
                return Poll::Ready(Some(val));
            } else if Rc::strong_count(&self.inner) == 1 {
                return Poll::Ready(None);
            }
            inner.waker = Some(cx.waker().clone());
            Poll::Pending
        })
    }
}

pub(super) struct NotifierIo<Io> {
    inner: Rc<_SharedIo<Io>>,
}

impl<Io> Drop for NotifierIo<Io> {
    fn drop(&mut self) {
        if let Some(waker) = self.inner.notify.borrow_mut().waker.take() {
            waker.wake();
        }
    }
}

impl<Io> NotifierIo<Io> {
    pub(super) fn io(&self) -> &Io {
        &self.inner.io
    }

    pub(super) fn notify(&mut self, val: BytesMut) {
        self.inner.notify.borrow_mut().val = Some(val);
    }
}

struct Inner<V> {
    waker: Option<Waker>,
    val: Option<V>,
}
