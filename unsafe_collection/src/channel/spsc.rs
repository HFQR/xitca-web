//! An async bounded spsc channel.

#![allow(clippy::non_send_fields_in_send_ty)]

extern crate alloc;

use core::{
    fmt,
    future::{poll_fn, Future},
    marker::PhantomData,
    mem::ManuallyDrop,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};

use alloc::{sync::Arc, vec::Vec};

use cache_padded::CachePadded;
use futures_core::task::__internal::AtomicWaker;

struct Inner<T> {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    buffer: *mut T,
    cap: usize,
    tx_waker: AtomicWaker,
    rx_waker: AtomicWaker,
    _marker: PhantomData<T>,
}

impl<T> Inner<T> {
    unsafe fn slot(&self, pos: usize) -> *mut T {
        if pos < self.cap {
            self.buffer.add(pos)
        } else {
            self.buffer.add(pos - self.cap)
        }
    }

    fn increment(&self, pos: usize) -> usize {
        if pos < 2 * self.cap - 1 {
            pos + 1
        } else {
            0
        }
    }

    fn distance(&self, a: usize, b: usize) -> usize {
        if a <= b {
            b - a
        } else {
            2 * self.cap - a + b
        }
    }
}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        let mut head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);

        // Loop over all slots that hold a value and drop them.
        while head != tail {
            unsafe {
                self.slot(head).drop_in_place();
            }
            head = self.increment(head);
        }

        // Finally, deallocate the buffer, but don't run any destructors.
        unsafe {
            Vec::from_raw_parts(self.buffer, 0, self.cap);
        }
    }
}

/// Creates a bounded spsc channel with the given capacity.
///
/// Returns the sender and receiver.
///
/// # Panics
///
/// Panics if the capacity is zero.
///
pub fn channel<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    assert!(cap > 0, "capacity must be non-zero");

    let inner = Arc::new(Inner {
        head: CachePadded::new(AtomicUsize::new(0)),
        tail: CachePadded::new(AtomicUsize::new(0)),
        buffer: ManuallyDrop::new(Vec::with_capacity(cap)).as_mut_ptr(),
        cap,
        tx_waker: AtomicWaker::new(),
        rx_waker: AtomicWaker::new(),
        _marker: PhantomData,
    });

    let tx = Sender {
        inner: inner.clone(),
        head: 0,
        tail: 0,
    };

    let rx = Receiver {
        inner,
        head: 0,
        tail: 0,
    };

    (tx, rx)
}

/// The sender part of a spsc channel.
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
    head: usize,
    tail: usize,
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("Sender { .. }")
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.inner.rx_waker.wake();
    }
}

unsafe impl<T: Send> Send for Sender<T> {}

unsafe impl<T: Send + Sync> Sync for Sender<T> {}

impl<T> Sender<T> {
    #[inline]
    pub fn try_send(&mut self, value: T) -> Result<(), SendError<T>> {
        match self.push(value) {
            Ok(_) => {
                self.inner.rx_waker.wake();
                Ok(())
            }
            Err(value) => Err(SendError::Full(value)),
        }
    }

    pub async fn send(&mut self, value: T) -> Result<(), SendError<T>> {
        struct SendFuture<'a, T> {
            tx: &'a mut Sender<T>,
            value: Option<T>,
        }

        impl<T> Future for SendFuture<'_, T> {
            type Output = Result<(), SendError<T>>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                // SAFETY:
                // This is safe as Self is not moved in the following code.
                let this = unsafe { self.get_unchecked_mut() };

                fn send_once<T, F>(sender: &mut Sender<T>, value: T, on_full: F) -> Poll<Result<(), SendError<T>>>
                where
                    F: FnOnce(&mut Sender<T>, T) -> Poll<Result<(), SendError<T>>>,
                {
                    match sender.try_send(value) {
                        Ok(_) => Poll::Ready(Ok(())),
                        Err(SendError::Full(value)) => on_full(sender, value),
                        Err(e) => Poll::Ready(Err(e)),
                    }
                }

                send_once(
                    this.tx,
                    this.value.take().expect("SendFuture polled after finished"),
                    |tx, value| {
                        tx.inner.tx_waker.register(cx.waker());
                        send_once(tx, value, |_, value| {
                            this.value = Some(value);
                            Poll::Pending
                        })
                    },
                )
            }
        }

        SendFuture {
            tx: self,
            value: Some(value),
        }
        .await
    }

    fn push(&mut self, value: T) -> Result<(), T> {
        let mut head = self.head;
        let mut tail = self.tail;

        // Check if the queue is *possibly* full.
        if self.inner.distance(head, tail) == self.inner.cap {
            // We need to refresh the head and check again if the queue is *really* full.
            head = self.inner.head.load(Ordering::Acquire);
            self.head = head;

            // Is the queue *really* full?
            if self.inner.distance(head, tail) == self.inner.cap {
                return Err(value);
            }
        }

        // Write the value into the tail slot.
        unsafe {
            self.inner.slot(tail).write(value);
        }

        // Move the tail one slot forward.
        tail = self.inner.increment(tail);
        self.inner.tail.store(tail, Ordering::Release);
        self.tail = tail;

        Ok(())
    }
}

/// The receiver of a bounded spsc channel.
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
    head: usize,
    tail: usize,
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("Receiver { .. }")
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}

impl<T> Receiver<T> {
    #[inline]
    pub async fn recv(&mut self) -> Option<T> {
        poll_fn(|cx| self.poll_recv(cx)).await
    }

    pub fn poll_recv(&mut self, cx: &mut Context) -> Poll<Option<T>> {
        fn recv_once<T, F>(rx: &mut Receiver<T>, on_none: F) -> Poll<Option<T>>
        where
            F: FnOnce(&mut Receiver<T>) -> Poll<Option<T>>,
        {
            match rx.pop() {
                Some(value) => {
                    rx.inner.tx_waker.wake();
                    Poll::Ready(Some(value))
                }
                None => {
                    if Arc::strong_count(&rx.inner) == 1 {
                        Poll::Ready(None)
                    } else {
                        on_none(rx)
                    }
                }
            }
        }

        recv_once(self, |this| {
            this.inner.rx_waker.register(cx.waker());
            recv_once(this, |_| Poll::Pending)
        })
    }

    fn pop(&mut self) -> Option<T> {
        let mut head = self.head;
        let mut tail = self.tail;

        // Check if the queue is *possibly* empty.
        if head == tail {
            // We need to refresh the tail and check again if the queue is *really* empty.
            tail = self.inner.tail.load(Ordering::Acquire);
            self.tail = tail;

            // Is the queue *really* empty?
            if head == tail {
                return None;
            }
        }

        // Read the value from the head slot.
        let value = unsafe { self.inner.slot(head).read() };

        // Move the head one slot forward.
        head = self.inner.increment(head);
        self.inner.head.store(head, Ordering::Release);
        self.head = head;

        Some(value)
    }
}

/// Error which occurs when channel is full or closed from receiver part.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum SendError<T> {
    Full(T),
    Closed(T),
}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Full(..) => write!(f, "SendError::Full(..)"),
            Self::Closed(..) => write!(f, "SendError::Closed(..)"),
        }
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Full(..) => write!(f, "SendError::Full(..)"),
            Self::Closed(..) => write!(f, "SendError::Closed(..)"),
        }
    }
}

#[cfg(test)]
mod test {
    use core::pin::pin;

    use crate::futures::noop_waker;

    use super::*;

    #[test]
    fn spsc() {
        let (mut tx, mut rx) = channel::<usize>(8);

        let waker = noop_waker();

        let cx = &mut Context::from_waker(&waker);

        for i in 0..8 {
            let fut = tx.send(i);
            assert!(pin!(fut).poll(cx).is_ready());
        }

        for i in 0..8 {
            let fut = rx.recv();
            match pin!(fut).poll(cx) {
                Poll::Ready(Some(i2)) => assert_eq!(i, i2),
                _ => unreachable!(),
            }
        }

        let fut = rx.recv();
        assert!(pin!(fut).poll(cx).is_pending());
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn race() {
        let (mut tx, mut rx) = channel(1);

        let (tx1, rx1) = tokio::sync::oneshot::channel::<()>();

        let h1 = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(async {
                    for i in 0..1024 {
                        tx.send(i).await.unwrap();
                    }
                    rx1.await.unwrap();
                })
        });

        let h2 = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(async {
                    for i in 0..1024 {
                        assert_eq!(rx.recv().await.unwrap(), i);
                    }

                    let handle = tokio::spawn(async move { rx.recv().await });

                    tokio::task::yield_now().await;

                    tx1.send(()).unwrap();

                    assert_eq!(handle.await.unwrap(), None);
                })
        });

        h1.join().unwrap();
        h2.join().unwrap();
    }

    #[test]
    fn drop() {
        let (mut tx, mut rx) = channel::<usize>(8);

        let waker = noop_waker();

        let cx = &mut Context::from_waker(&waker);

        {
            let fut = tx.send(996);
            assert!(pin!(fut).poll(cx).is_ready());
        }

        {
            {
                let fut = rx.recv();
                match pin!(fut).poll(cx) {
                    Poll::Ready(Some(i)) => assert_eq!(i, 996),
                    _ => unreachable!(),
                }
            }

            let fut = rx.recv();

            assert!(pin!(fut).poll(cx).is_pending());

            let _tx = tx;
        }

        let fut = rx.recv();
        let fut = pin!(fut);
        match fut.poll(cx) {
            Poll::Ready(None) => {}
            _ => unreachable!(),
        }
    }
}
