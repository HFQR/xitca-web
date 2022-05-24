//! An async bounded spsc channel.

#![allow(clippy::non_send_fields_in_send_ty)]

extern crate alloc;

use core::{
    cell::UnsafeCell,
    fmt,
    future::Future,
    marker::PhantomData,
    mem,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

use alloc::{sync::Arc, vec::Vec};

use cache_padded::CachePadded;

struct Inner<T> {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    buffer: *mut T,
    cap: usize,
    waker: TryAtomicWaker,
    _marker: PhantomData<T>,
}

impl<T> Inner<T> {
    #[inline]
    unsafe fn slot(&self, pos: usize) -> *mut T {
        if pos < self.cap {
            self.buffer.add(pos)
        } else {
            self.buffer.add(pos - self.cap)
        }
    }

    #[inline]
    fn increment(&self, pos: usize) -> usize {
        if pos < 2 * self.cap - 1 {
            pos + 1
        } else {
            0
        }
    }

    #[inline]
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

    let buffer = {
        let mut v = Vec::<T>::with_capacity(cap);
        let ptr = v.as_mut_ptr();
        mem::forget(v);
        ptr
    };

    let inner = Arc::new(Inner {
        head: CachePadded::new(AtomicUsize::new(0)),
        tail: CachePadded::new(AtomicUsize::new(0)),
        buffer,
        cap,
        waker: TryAtomicWaker::default(),
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

unsafe impl<T: Send> Send for Sender<T> {}

impl<T> Sender<T> {
    pub async fn send(&mut self, value: T) -> Result<(), SendError<T>> {
        struct SendFuture<'a, T> {
            producer: &'a mut Sender<T>,
            value: Option<T>,
        }

        impl<T> Future for SendFuture<'_, T> {
            type Output = Result<(), SendError<T>>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                // SAFETY:
                // This is safe as Self is not moved in the following code.
                let this = unsafe { self.get_unchecked_mut() };

                match this
                    .producer
                    .push(this.value.take().expect("SendFuture polled after finished"))
                {
                    Ok(_) => {
                        this.producer.inner.waker.try_wake();
                        Poll::Ready(Ok(()))
                    }
                    Err(value) => {
                        this.producer.inner.waker.try_register(cx.waker());
                        this.value = Some(value);
                        Poll::Pending
                    }
                }
            }
        }

        SendFuture {
            producer: self,
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

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("Sender { .. }")
    }
}

/// The receiver of a bounded single-producer single-consumer queue.
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
    head: usize,
    tail: usize,
}

unsafe impl<T: Send> Send for Receiver<T> {}

impl<T> Receiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        struct RecvFuture<'a, T> {
            consumer: &'a mut Receiver<T>,
        }

        impl<T> Future for RecvFuture<'_, T> {
            type Output = Option<T>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                // SAFETY:
                // This is safe as Self is not moved in the following code.
                let this = unsafe { self.get_unchecked_mut() };

                match this.consumer.pop() {
                    Some(value) => Poll::Ready(Some(value)),
                    None => {
                        this.consumer.inner.waker.try_register(cx.waker());
                        Poll::Pending
                    }
                }
            }
        }

        RecvFuture { consumer: self }.await
    }

    /// Attempts to pop an element from the queue.
    ///
    /// If the queue is empty, an error is returned.
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

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("Receiver { .. }")
    }
}

/// Error which occurs when pushing into a full queue.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "SendError(..)".fmt(f)
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "Receiver is closed".fmt(f)
    }
}

// an atomic waker only try to register when there is no other operating on the atomic state.
struct TryAtomicWaker {
    state: CachePadded<AtomicUsize>,
    waker: UnsafeCell<Option<Waker>>,
}

impl Default for TryAtomicWaker {
    fn default() -> Self {
        TryAtomicWaker::new()
    }
}

impl fmt::Debug for TryAtomicWaker {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "AtomicWaker")
    }
}

unsafe impl Send for TryAtomicWaker {}

unsafe impl Sync for TryAtomicWaker {}

const NONE: usize = 0;
const OPERATING: usize = 1;
const SOME: usize = 1 << 1;

impl TryAtomicWaker {
    fn new() -> TryAtomicWaker {
        TryAtomicWaker {
            state: CachePadded::new(AtomicUsize::new(NONE)),
            waker: UnsafeCell::new(None),
        }
    }

    fn try_register(&self, waker: &Waker) {
        let mut state = self.state.load(Ordering::Relaxed);

        while state != OPERATING {
            match self
                .state
                .compare_exchange_weak(state, OPERATING, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.register(waker);
                    return self.state.store(SOME, Ordering::Release);
                }
                Err(s) => state = s,
            }
        }

        // schedule task for wake up and try later.
        waker.wake_by_ref();
    }

    fn try_wake(&self) {
        let mut state = self.state.load(Ordering::Relaxed);

        while state == SOME {
            match self
                .state
                .compare_exchange_weak(state, OPERATING, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.wake();
                    self.state.store(NONE, Ordering::Release);
                    return;
                }
                Err(s) => state = s,
            }
        }
    }

    fn register(&self, waker: &Waker) {
        // SAFETY:
        //
        // deref is guarded by atomic state and exclusive to the only writer that successfully
        // write OPERATING state.
        let set = unsafe { &mut *self.waker.get() };
        if let Some(waker) = set.replace(waker.clone()) {
            waker.wake();
        }
    }

    fn wake(&self) {
        // SAFETY:
        //
        // deref is guarded by atomic state and exclusive to the only writer that successfully
        // write OPERATING state.
        let set = unsafe { &mut *self.waker.get() };
        set.take().unwrap().wake();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use alloc::{sync::Arc, task::Wake};

    struct DummyWaker;

    impl Wake for DummyWaker {
        fn wake(self: Arc<Self>) {
            // do nothing.
        }
    }

    #[test]
    fn spsc() {
        let (mut tx, mut rx) = channel::<usize>(8);

        let waker = Waker::from(Arc::new(DummyWaker));

        let cx = &mut Context::from_waker(&waker);

        for i in 0..8 {
            let mut fut = tx.send(i);
            assert!(unsafe { Pin::new_unchecked(&mut fut) }.poll(cx).is_ready());
        }

        for i in 0..8 {
            let mut fut = rx.recv();
            match unsafe { Pin::new_unchecked(&mut fut) }.poll(cx) {
                Poll::Ready(Some(i2)) => assert_eq!(i, i2),
                _ => unreachable!(),
            }
        }

        let mut fut = rx.recv();
        assert!(unsafe { Pin::new_unchecked(&mut fut) }.poll(cx).is_pending());
    }
}
