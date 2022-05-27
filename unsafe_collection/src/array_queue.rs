//! A simple stack ring buffer with FIFO queue.

use core::{fmt, mem::MaybeUninit};

use super::uninit::uninit_array;

pub struct ArrayQueue<T, const N: usize> {
    inner: [MaybeUninit<T>; N],
    next: usize,
    len: usize,
}

impl<T, const N: usize> ArrayQueue<T, N> {
    pub const fn new() -> Self {
        ArrayQueue {
            inner: uninit_array(),
            next: 0,
            len: 0,
        }
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub const fn is_full(&self) -> bool {
        self.len == N
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    // SAFETY:
    // caller must make sure given index is not out of bound and properly initialized.
    unsafe fn read(&mut self, idx: usize) -> T {
        self.inner.get_unchecked(idx).assume_init_read()
    }

    // SAFETY:
    // caller must make sure given index is not out of bound and properly initialized.
    unsafe fn get_unchecked(&self, idx: usize) -> &T {
        self.inner.get_unchecked(idx).assume_init_ref()
    }

    // SAFETY:
    // caller must make sure given index is not out of bound and properly initialized.
    unsafe fn get_unchecked_mut(&mut self, idx: usize) -> &mut T {
        self.inner.get_unchecked_mut(idx).assume_init_mut()
    }

    // SAFETY:
    // caller must make sure given index is not out of bound.
    unsafe fn write(&mut self, idx: usize, value: T) {
        self.inner.get_unchecked_mut(idx).write(value);
    }

    fn incr_tail_len(&mut self) {
        self.next += 1;
        self.len += 1;

        if self.next == N {
            self.next = 0;
        }
    }

    pub fn front(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            let idx = self.front_idx();

            Some(unsafe { self.get_unchecked(idx) })
        }
    }

    pub fn front_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            None
        } else {
            let idx = self.front_idx();

            Some(unsafe { self.get_unchecked_mut(idx) })
        }
    }

    pub fn clear(&mut self) {
        while self.pop_front().is_some() {}
        self.next = 0;
    }

    pub fn pop_front(&mut self) -> Option<T> {
        if self.is_empty() {
            None
        } else {
            let idx = self.front_idx();
            self.len -= 1;

            unsafe { Some(self.read(idx)) }
        }
    }

    const fn front_idx(&self) -> usize {
        if self.next >= self.len {
            self.next - self.len
        } else {
            N + self.next - self.len
        }
    }

    pub fn push_back(&mut self, value: T) -> Result<(), PushError<T>> {
        if self.is_full() {
            return Err(PushError(value));
        }

        unsafe { self.write(self.next, value) };

        self.incr_tail_len();

        Ok(())
    }

    pub const fn iter(&self) -> Iter<'_, T, N> {
        Iter {
            queue: self,
            tail: self.next,
            len: self.len(),
        }
    }
}

pub struct PushError<T>(T);

impl<T> PushError<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for PushError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PushError(..)")
    }
}

impl<T, const N: usize> fmt::Debug for ArrayQueue<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArrayQueue")
    }
}

impl<T, const N: usize> Drop for ArrayQueue<T, N> {
    fn drop(&mut self) {
        self.clear()
    }
}

#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
#[derive(Clone)]
pub struct Iter<'a, T, const N: usize> {
    queue: &'a ArrayQueue<T, N>,
    tail: usize,
    len: usize,
}

impl<'a, T, const N: usize> Iterator for Iter<'a, T, N> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        if self.len == 0 {
            return None;
        }

        let idx = if self.tail >= self.len {
            self.tail - self.len
        } else {
            N + self.tail - self.len
        };

        self.len -= 1;

        unsafe { Some(self.queue.get_unchecked(idx)) }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn iterate() {
        let mut queue = ArrayQueue::<_, 5>::new();

        queue.push_back("996").ok().unwrap();
        queue.push_back("231").ok().unwrap();
        queue.push_back("007").ok().unwrap();

        let mut iter = queue.iter();

        assert_eq!(iter.next(), Some(&"996"));
        assert_eq!(iter.next(), Some(&"231"));
        assert_eq!(iter.next(), Some(&"007"));
        assert_eq!(iter.next(), None);

        assert_eq!(queue.pop_front(), Some("996"));

        let mut iter = queue.iter();

        assert_eq!(iter.next(), Some(&"231"));
        assert_eq!(iter.next(), Some(&"007"));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn cap() {
        let mut queue = ArrayQueue::<_, 3>::new();

        queue.push_back("996").ok().unwrap();
        queue.push_back("231").ok().unwrap();
        queue.push_back("007").ok().unwrap();

        assert!(queue.push_back("123").is_err());

        assert_eq!(queue.pop_front(), Some("996"));
        queue.push_back("123").unwrap();
        assert!(queue.push_back("123").is_err());

        assert_eq!(queue.pop_front(), Some("231"));
        assert_eq!(queue.pop_front(), Some("007"));
        queue.push_back("123").unwrap();
        queue.push_back("123").unwrap();
        assert!(queue.push_back("123").is_err());
    }

    #[test]
    fn front_mut() {
        let mut queue = ArrayQueue::<_, 3>::new();

        assert_eq!(None, queue.front_mut());

        queue.push_back("996").ok().unwrap();
        queue.push_back("231").ok().unwrap();
        queue.push_back("007").ok().unwrap();

        assert_eq!(Some(&mut "996"), queue.front_mut());

        queue.pop_front();
        assert_eq!(Some(&mut "231"), queue.front_mut());

        queue.pop_front();
        assert_eq!(Some(&mut "007"), queue.front_mut());

        queue.pop_front();
        assert_eq!(None, queue.front_mut());
    }

    #[test]
    fn wrap() {
        let mut queue = ArrayQueue::<_, 4>::new();

        for i in 0..4 {
            assert!(queue.push_back(i).is_ok());
        }

        assert!(queue.is_full());

        assert!(queue.pop_front().is_some());
        assert!(queue.pop_front().is_some());

        assert!(queue.push_back(1).is_ok());

        queue.clear();
    }

    #[test]
    fn drop() {
        extern crate alloc;

        use alloc::sync::Arc;

        let item = Arc::new(123);

        {
            let _queue = ArrayQueue::<u8, 3>::new();
        }

        {
            let mut queue = ArrayQueue::<_, 3>::new();

            queue.push_back(item.clone()).ok().unwrap();
            queue.push_back(item.clone()).ok().unwrap();

            assert_eq!(Arc::strong_count(&item), 3);
        }

        assert_eq!(Arc::strong_count(&item), 1);

        {
            let mut queue = ArrayQueue::<_, 3>::new();

            queue.push_back(item.clone()).ok().unwrap();

            assert_eq!(Arc::strong_count(&item), 2);
        }

        assert_eq!(Arc::strong_count(&item), 1);

        {
            let mut queue = ArrayQueue::<_, 3>::new();

            queue.push_back(item.clone()).ok().unwrap();
            queue.push_back(item.clone()).ok().unwrap();
            queue.push_back(item.clone()).ok().unwrap();

            assert_eq!(Arc::strong_count(&item), 4);
        }

        assert_eq!(Arc::strong_count(&item), 1);
    }
}
