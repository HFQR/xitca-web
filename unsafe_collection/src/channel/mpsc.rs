//! An async aware mpsc array.

extern crate alloc;

use core::{
    cell::RefCell,
    fmt,
    future::Future,
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use alloc::rc::Rc;

use std::collections::linked_list::LinkedList;

use crate::futures::poll_fn;

/// An async array that act in mpsc manner. There can be multiple `Sender`s and one `Receiver`.
pub fn async_vec<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    assert!(cap > 0, "async_vec must have a capacity larger than 0.");

    let array = Rc::new(RefCell::new(AsyncVec::new(cap)));

    (Sender { inner: array.clone() }, Receiver { inner: array })
}

#[derive(Clone)]
pub struct Sender<T> {
    inner: Rc<RefCell<AsyncVec<T>>>,
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // Last copy of Sender. wake up receiver.
        if Rc::strong_count(&self.inner) == 2 {
            let mut inner = self.inner.borrow_mut();
            inner.set_close();
            inner.wake_receiver();
        }
    }
}

impl<T> Sender<T> {
    pub fn is_closed(&self) -> bool {
        self.inner.borrow_mut().is_closed()
    }

    pub async fn send(&self, value: T) -> Result<(), Error<T>> {
        struct SendFuture<'a, T> {
            sender: &'a Sender<T>,
            value: Option<T>,
        }

        impl<T> Unpin for SendFuture<'_, T> {}

        impl<T> Future for SendFuture<'_, T> {
            type Output = Result<(), Error<T>>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let this = self.get_mut();

                match this.value.take() {
                    Some(value) => {
                        let mut inner = this.sender.inner.borrow_mut();
                        match inner.try_push(value) {
                            Ok(_) => {
                                inner.wake_receiver();
                                Poll::Ready(Ok(()))
                            }
                            Err(Error::Full(value)) => {
                                this.value = Some(value);
                                inner.register_sender_waker(cx.waker());
                                Poll::Pending
                            }
                            Err(e) => Poll::Ready(Err(e)),
                        }
                    }
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

pub struct Receiver<T> {
    inner: Rc<RefCell<AsyncVec<T>>>,
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.set_close();
        while let Some(waker) = inner.sender_waker.pop_front() {
            waker.wake();
        }
    }
}

impl<T> Receiver<T> {
    /// wait for items to be available.
    /// When this future yields the receiver can peek into the array and advance it.
    pub fn wait(&mut self) -> impl Future<Output = Result<(), Error<T>>> + '_ {
        poll_fn(|cx| {
            let mut array = self.inner.borrow_mut();
            if array.is_empty() {
                if array.is_closed() {
                    Poll::Ready(Err(Error::SenderClosed))
                } else {
                    array.register_receiver_waker(cx.waker());
                    Poll::Pending
                }
            } else {
                Poll::Ready(Ok(()))
            }
        })
    }

    /// iter through available items inside array.
    #[inline]
    pub fn with_iter<F, O>(&mut self, func: F) -> O
    where
        F: for<'i> FnOnce(Iter<'i, T>) -> O,
    {
        let mut inner = self.inner.borrow_mut();
        func(inner.iter())
    }

    /// Advance the array by iterate the array and pop drop items when given closure returns `true`.
    pub fn advance_until<F>(&mut self, func: F)
    where
        F: FnMut(&mut T) -> bool,
    {
        let mut inner = self.inner.borrow_mut();
        let count = inner.advance_until(func);
        inner.wake_sender(count);
    }
}

struct AsyncVec<T> {
    // TODO: use a more efficient list.
    sender_waker: LinkedList<Waker>,
    receiver_waker: Option<Waker>,
    array: *mut T,
    cap: usize,
    len: usize,
    tail: usize,
    closed: bool,
}

impl<T> AsyncVec<T> {
    fn new(cap: usize) -> Self {
        Self {
            sender_waker: LinkedList::new(),
            receiver_waker: None,
            array: ManuallyDrop::new(Vec::with_capacity(cap)).as_mut_ptr(),
            cap,
            len: 0,
            tail: 0,
            closed: false,
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn is_full(&self) -> bool {
        self.len == self.cap
    }

    fn set_close(&mut self) {
        self.closed = true;
    }

    fn is_closed(&self) -> bool {
        self.closed
    }

    fn iter(&mut self) -> Iter<'_, T> {
        Iter {
            vec: self,
            tail: self.tail,
            len: self.len,
        }
    }

    fn advance_until<F>(&mut self, mut func: F) -> usize
    where
        F: FnMut(&mut T) -> bool,
    {
        let mut count = 0;

        while let Some(value) = self.front_mut() {
            match func(value) {
                true => {
                    count += 1;
                    // SAFETY:
                    // only reachable when front is Some.
                    unsafe {
                        self.pop_front_unchecked();
                    }
                }
                false => break,
            }
        }

        count
    }

    fn try_push(&mut self, value: T) -> Result<(), Error<T>> {
        if self.is_closed() {
            Err(Error::ReceiverClosed(value))
        } else if self.is_full() {
            Err(Error::Full(value))
        } else {
            // SAFETY:
            // just checked self is not full.
            unsafe { self.push_back_unchecked(value) };
            Ok(())
        }
    }

    fn wake_receiver(&mut self) {
        if let Some(waker) = self.receiver_waker.take() {
            waker.wake();
        }
    }

    fn wake_sender(&mut self, count: usize) {
        for _ in 0..count {
            match self.sender_waker.pop_front() {
                Some(waker) => waker.wake(),
                None => return,
            }
        }
    }

    fn register_receiver_waker(&mut self, waker: &Waker) {
        self.receiver_waker = Some(waker.clone());
    }

    fn register_sender_waker(&mut self, waker: &Waker) {
        if let Some(node) = self.sender_waker.iter_mut().find(|w| w.will_wake(waker)) {
            *node = waker.clone();
        } else {
            self.sender_waker.push_back(waker.clone());
        }
    }

    fn front_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            None
        } else {
            let idx = self.front_idx();

            // SAFETY:
            // idx is guaranteed to be in range and contain T.
            Some(unsafe { self.get_mut_unchecked(idx) })
        }
    }

    fn pop_front(&mut self) -> Option<T> {
        if self.is_empty() {
            None
        } else {
            // SAFETY:
            // just checked self is not empty.
            unsafe { Some(self.pop_front_unchecked()) }
        }
    }

    fn front_idx(&self) -> usize {
        if self.tail >= self.len {
            self.tail - self.len
        } else {
            self.cap + self.tail - self.len
        }
    }

    // SAFETY:
    // caller must make sure self is not empty.
    unsafe fn pop_front_unchecked(&mut self) -> T {
        let idx = self.front_idx();
        self.len -= 1;
        self.read_unchecked(idx)
    }

    // SAFETY:
    // caller must make sure self is not full.
    unsafe fn push_back_unchecked(&mut self, value: T) {
        self.write_unchecked(self.tail, value);
        self.tail += 1;
        self.len += 1;

        if self.tail == self.cap {
            self.tail = 0;
        }
    }

    // SAFETY:
    // caller must make sure given index is not out of bound and properly initialized.
    unsafe fn get_unchecked(&self, idx: usize) -> &T {
        &*self.array.add(idx)
    }

    // SAFETY:
    // caller must make sure given index is not out of bound and properly initialized.
    unsafe fn get_mut_unchecked(&mut self, idx: usize) -> &mut T {
        &mut *self.array.add(idx)
    }

    // SAFETY:
    // caller must make sure given index is not out of bound.
    unsafe fn write_unchecked(&mut self, idx: usize, value: T) {
        self.array.add(idx).write(value);
    }

    // SAFETY:
    // caller must make sure given index is not out of bound and properly initialized.
    unsafe fn read_unchecked(&mut self, idx: usize) -> T {
        self.array.add(idx).read()
    }
}

impl<T> Drop for AsyncVec<T> {
    fn drop(&mut self) {
        while self.pop_front().is_some() {}
        // SAFETY:
        // deallocate the pointer but don't drop anything. (just did by popping everything).
        unsafe {
            Vec::from_raw_parts(self.array, 0, self.cap);
        }
    }
}

#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
#[derive(Clone)]
pub struct Iter<'a, T> {
    vec: &'a AsyncVec<T>,
    tail: usize,
    len: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        if self.len == 0 {
            return None;
        }

        let idx = if self.tail >= self.len {
            self.tail - self.len
        } else {
            self.vec.cap + self.tail - self.len
        };

        self.len -= 1;

        unsafe { Some(self.vec.get_unchecked(idx)) }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

pub enum Error<T> {
    SenderClosed,
    ReceiverClosed(T),
    Full(T),
}

impl<T> Error<T> {
    pub fn into_inner(self) -> T {
        match self {
            Self::ReceiverClosed(value) => value,
            _ => unreachable!("Can not retrieve data from Error"),
        }
    }
}

impl<T> fmt::Debug for Error<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SenderClosed => write!(f, "SenderClosed"),
            Self::ReceiverClosed(_) => write!(f, "ReceiverClosed(..)"),
            Self::Full(_) => write!(f, "Full(..)"),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use alloc::{sync::Arc, task::Wake};

    use crate::pin;

    #[test]
    fn push() {
        let mut vec = AsyncVec::new(3);

        assert!(vec.try_push(1).is_ok());
        let mut iter = vec.iter();
        assert_eq!(iter.next(), Some(&1));
        assert_eq!(iter.next(), None);

        assert!(vec.try_push(2).is_ok());
        let mut iter = vec.iter();
        assert_eq!(iter.next(), Some(&1));
        assert_eq!(iter.next(), Some(&2));
        assert_eq!(iter.next(), None);

        assert!(vec.try_push(3).is_ok());
        let mut iter = vec.iter();
        assert_eq!(iter.next(), Some(&1));
        assert_eq!(iter.next(), Some(&2));
        assert_eq!(iter.next(), Some(&3));
        assert_eq!(iter.next(), None);

        assert!(vec.try_push(4).is_err());
        let mut iter = vec.iter();
        assert_eq!(iter.next(), Some(&1));
        assert_eq!(iter.next(), Some(&2));
        assert_eq!(iter.next(), Some(&3));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn advance() {
        let mut v = AsyncVec::new(3);

        assert!(v.try_push(1).is_ok());
        assert!(v.try_push(2).is_ok());
        assert!(v.try_push(3).is_ok());

        let v2 = v.iter().collect::<Vec<&usize>>();
        assert_eq!(v2, &[&1, &2, &3]);

        let count = v.advance_until(|i| *i != 3);
        assert_eq!(count, 2);

        let v2 = v.iter().collect::<Vec<&usize>>();
        assert_eq!(v2, &[&3]);

        assert!(v.try_push(4).is_ok());
        let v2 = v.iter().collect::<Vec<&usize>>();
        assert_eq!(v2, &[&3, &4]);
    }

    #[test]
    fn drop() {
        let counter = Arc::new(());

        {
            let mut v = AsyncVec::new(3);

            assert!(v.try_push(counter.clone()).is_ok());
            assert!(v.try_push(counter.clone()).is_ok());

            assert_eq!(Arc::strong_count(&counter), 3);
        }

        assert_eq!(Arc::strong_count(&counter), 1);

        {
            let mut v = AsyncVec::new(3);

            assert!(v.try_push(counter.clone()).is_ok());
            assert!(v.try_push(counter.clone()).is_ok());

            {
                let _ = v.pop_front();
            }

            assert!(v.try_push(counter.clone()).is_ok());

            assert_eq!(Arc::strong_count(&counter), 3);
        }

        assert_eq!(Arc::strong_count(&counter), 1);
    }

    struct DummyWaker;

    impl Wake for DummyWaker {
        fn wake(self: Arc<Self>) {
            // do nothing.
        }
    }

    #[test]
    fn mpsc() {
        let (tx, mut rx) = async_vec(8);

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

        rx.with_iter(|iter| {
            let items = iter.collect::<Vec<&usize>>();

            assert_eq!(items.len(), 8);
            assert_eq!(items[3], &3);
        });

        rx.advance_until(|i| *i < 8);

        let fut = rx.wait();
        pin!(fut);
        assert!(fut.poll(cx).is_pending());
    }
}
