//! An async aware mpsc array.

extern crate alloc;

use core::{
    cell::UnsafeCell,
    future::Future,
    pin::Pin,
    slice,
    sync::atomic::{fence, AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

use alloc::sync::Arc;
use cache_padded::CachePadded;

use std::{collections::linked_list::LinkedList, sync::Mutex};

use crate::{
    futures::poll_fn,
    uninit::{self, UninitArray},
    waker::AtomicWaker,
};

/// An async array that act in mpsc manner. There can be multiple `Sender`s and one `Receiver`.
pub fn async_array<T, const N: usize>() -> (Sender<T, N>, Receiver<T, N>) {
    let array = Arc::new(AsyncArray {
        sender_waker: Mutex::new(LinkedList::new()),
        receiver_waker: AtomicWaker::new(),
        array: AtomicArray::new(),
    });

    (Sender { inner: array.clone() }, Receiver { inner: array })
}

pub struct Sender<T, const N: usize> {
    inner: Arc<AsyncArray<T, N>>,
}

impl<T, const N: usize> Sender<T, N> {
    pub async fn send(&self, value: T) {
        struct SendFuture<'a, T, const N: usize> {
            sender: &'a Sender<T, N>,
            value: Option<T>,
        }

        impl<T, const N: usize> Unpin for SendFuture<'_, T, N> {}

        impl<T, const N: usize> Future for SendFuture<'_, T, N> {
            type Output = ();

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let this = self.get_mut();

                match this.value.take() {
                    Some(value) => match this.sender.inner.array.push_back(value) {
                        Ok(_) => {
                            this.sender.inner.receiver_waker.wake();
                            Poll::Ready(())
                        }
                        Err(value) => {
                            this.value = Some(value);

                            let mut waiters = this.sender.inner.sender_waker.lock().unwrap();

                            let waker = cx.waker().clone();
                            if let Some(node) = waiters.iter_mut().find(|w| w.will_wake(cx.waker())) {
                                *node = waker;
                            } else {
                                waiters.push_back(waker);
                            }

                            Poll::Pending
                        }
                    },
                    None => panic!("SendFuture polled after finish"),
                }
            }
        }

        SendFuture {
            sender: self,
            value: Some(value),
        }
        .await
    }
}

pub struct Receiver<T, const N: usize> {
    inner: Arc<AsyncArray<T, N>>,
}

impl<T, const N: usize> Drop for Receiver<T, N> {
    fn drop(&mut self) {
        let mut waiters = self.inner.sender_waker.lock().unwrap();

        while let Some(waker) = waiters.pop_front() {
            waker.wake();
        }
    }
}

impl<T, const N: usize> Receiver<T, N> {
    /// wait for items to be available.
    /// When this future yields the receiver can peek into the array and advance it.
    pub fn wait(&mut self) -> impl Future<Output = ()> + '_ {
        poll_fn(|cx| {
            if self.is_empty() {
                self.inner.receiver_waker.register(cx.waker());
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
    }

    #[inline]
    pub fn is_empty(&mut self) -> bool {
        self.with_slice(|a, b| a.is_empty() && b.is_empty())
    }

    /// peek into the available items inside array.
    ///
    /// # Order:
    /// The two slices are presented in FIFO order. The items in first slice are always the ones
    /// added earlier into the array. The items in each slice follow the same ordering that early
    /// data always have smaller index than late ones.
    #[inline]
    pub fn with_slice<F, O>(&mut self, func: F) -> O
    where
        F: FnOnce(&[T], &[T]) -> O,
    {
        self.inner.array.with_slice(func)
    }

    /// Advance the array by iterate the array and pop drop items when given closure returns `true`.
    pub fn advance_until<F>(&mut self, func: F)
    where
        F: FnMut(&mut T) -> bool,
    {
        let count = self.inner.array.advance_until(func);

        let mut waiters = self.inner.sender_waker.lock().unwrap();

        for _ in 0..count {
            if let Some(waker) = waiters.pop_front() {
                waker.wake();
            } else {
                return;
            }
        }
    }
}

struct AsyncArray<T, const N: usize> {
    // TODO: use a more efficient list.
    sender_waker: Mutex<LinkedList<Waker>>,
    receiver_waker: AtomicWaker,
    array: AtomicArray<T, N>,
}

struct AtomicArray<T, const N: usize> {
    inner: UnsafeCell<UninitArray<T, N>>,
    next: CachePadded<AtomicUsize>,
    len: CachePadded<AtomicUsize>,
}

impl<T, const N: usize> Drop for AtomicArray<T, N> {
    fn drop(&mut self) {
        while self.len.load(Ordering::Relaxed) > 0 {
            self.advance_until(|_| true);
        }
    }
}

unsafe impl<T, const N: usize> Send for AtomicArray<T, N> where T: Send {}
unsafe impl<T, const N: usize> Sync for AtomicArray<T, N> where T: Sync {}

impl<T, const N: usize> AtomicArray<T, N> {
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(UninitArray::new()),
            next: CachePadded::new(AtomicUsize::new(0)),
            len: CachePadded::new(AtomicUsize::new(0)),
        }
    }

    fn push_back(&self, value: T) -> Result<(), T> {
        let mut len = self.len.load(Ordering::Relaxed);

        loop {
            if len == N {
                return Err(value);
            }

            match self
                .len
                .compare_exchange_weak(len, len + 1, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => {
                    let t = self.next.fetch_add(1, Ordering::Relaxed);

                    let idx = t % N;

                    // SAFETY:
                    // This is safe because idx is always in bound of N.
                    unsafe {
                        self.get_inner_mut().write_unchecked(idx, value);
                    }

                    fence(Ordering::Release);

                    return Ok(());
                }
                Err(l) => len = l,
            }
        }
    }

    fn with_slice<F, O>(&self, func: F) -> O
    where
        F: FnOnce(&[T], &[T]) -> O,
    {
        let len = self.len.load(Ordering::Relaxed);
        let idx = self.next.load(Ordering::Relaxed);

        let tail = idx % N;

        // SAFETY:
        //
        // tail and len must correctly track the initialized elements inside array.
        unsafe {
            if tail >= len {
                func(self.get_slice_unchecked(tail - len, len), &[])
            } else {
                let off = len - tail;

                func(
                    self.get_slice_unchecked(N - off, off),
                    self.get_slice_unchecked(0, tail),
                )
            }
        }
    }

    fn advance_until<F>(&self, mut func: F) -> usize
    where
        F: FnMut(&mut T) -> bool,
    {
        let mut count = 0;

        let mut len = self.len.load(Ordering::Acquire);

        loop {
            let idx = self.next.load(Ordering::Relaxed);

            let tail = idx % N;

            let head = if tail >= len { tail - len } else { N + tail - len };

            // SAFETY:
            //
            // idx and len must correctly check the initialized items and their index.
            unsafe {
                let mut value = self.get_inner_mut().read_unchecked(head);

                if func(&mut value) {
                    count += 1;

                    len = self.len.fetch_sub(1, Ordering::AcqRel) - 1;

                    if len == 0 {
                        return count;
                    }
                } else {
                    self.get_inner_mut().write_unchecked(head, value);
                    fence(Ordering::Release);
                    return count;
                }
            }
        }
    }

    // SAFETY:
    // UnsafeCell magic.
    #[allow(clippy::mut_from_ref)]
    unsafe fn get_inner_mut(&self) -> &mut UninitArray<T, N> {
        &mut *self.inner.get()
    }

    // SAFETY:
    // UnsafeCell magic.
    unsafe fn get_inner(&self) -> &UninitArray<T, N> {
        &*self.inner.get()
    }

    // SAFETY:
    // caller must make sure given slice info is not out of bound and properly initialized.
    unsafe fn get_slice_unchecked(&self, start: usize, len: usize) -> &[T] {
        let ptr = self.get_inner().as_ptr().add(start);
        let slice = slice::from_raw_parts(ptr, len);
        uninit::slice_assume_init(slice)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use core::task::Waker;

    use alloc::task::Wake;

    use crate::pin;

    #[test]
    fn push() {
        let array = AtomicArray::<usize, 3>::new();

        assert!(array.push_back(1).is_ok());

        array.with_slice(|a, b| {
            assert_eq!(a, &[1]);
            assert_eq!(b, &[]);
        });

        assert!(array.push_back(2).is_ok());

        array.with_slice(|a, b| {
            assert_eq!(a, &[1, 2]);
            assert_eq!(b, &[]);
        });

        assert!(array.push_back(3).is_ok());

        array.with_slice(|a, b| {
            assert_eq!(a, &[1, 2, 3]);
            assert_eq!(b, &[]);
        });

        assert!(array.push_back(4).is_err());
    }

    #[test]
    fn advance() {
        let array = AtomicArray::<usize, 3>::new();

        assert!(array.push_back(1).is_ok());
        assert!(array.push_back(2).is_ok());
        assert!(array.push_back(3).is_ok());

        array.with_slice(|a, b| {
            assert_eq!(a, &[1, 2, 3]);
            assert_eq!(b, &[]);
        });

        let _ = array.advance_until(|i| *i != 3);

        array.with_slice(|a, b| {
            assert_eq!(a, &[3]);
            assert_eq!(b, &[]);
        });

        assert!(array.push_back(4).is_ok());

        array.with_slice(|a, b| {
            assert_eq!(a, &[3]);
            assert_eq!(b, &[4]);
        });
    }

    struct DummyWaker;

    impl Wake for DummyWaker {
        fn wake(self: Arc<Self>) {
            // do nothing.
        }
    }

    #[test]
    fn spsc() {
        let (tx, mut rx) = async_array::<usize, 8>();

        let waker = Waker::from(Arc::new(DummyWaker));

        let cx = &mut Context::from_waker(&waker);

        for i in 0..8 {
            let fut = tx.send(i);
            pin!(fut);
            assert!(fut.poll(cx).is_ready());
        }

        {
            let fut = rx.wait();
            pin!(fut);
            assert!(fut.poll(cx).is_ready());
        }

        rx.with_slice(|a, b| {
            assert_eq!(a.len(), 8);
            assert_eq!(b.len(), 0);
            assert_eq!(a[3], 3);
        });

        rx.advance_until(|i| *i < 8);

        let fut = rx.wait();
        pin!(fut);
        assert!(fut.poll(cx).is_pending());
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn race() {
        let (tx, mut rx) = async_array::<u32, 8>();

        let (tx1, rx1) = tokio::sync::oneshot::channel::<()>();

        let h1 = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(async {
                    for i in 0..1024 {
                        tx.send(i).await;
                    }
                    rx1.await.unwrap();
                })
        });

        let h2 = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(async {
                    let mut outcome = 0;
                    while outcome < 1024 - 1 {
                        rx.wait().await;
                        rx.with_slice(|a, b| {
                            for i in a.iter().chain(b.iter()) {
                                outcome = core::cmp::max(outcome, *i);
                            }
                        });
                        rx.advance_until(|i| *i <= outcome);
                    }
                    tx1.send(()).unwrap();
                })
        });

        h1.join().unwrap();
        h2.join().unwrap();
    }
}
