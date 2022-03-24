use std::{fmt, mem::MaybeUninit};

pub struct ArrayQueue<T, const N: usize> {
    inner: [MaybeUninit<T>; N],
    head: usize,
    tail: usize,
    is_full: bool,
}

impl<T, const N: usize> ArrayQueue<T, N> {
    pub const fn new() -> Self {
        ArrayQueue {
            // SAFETY:
            // initialize MaybeUninit array is safe.
            inner: unsafe { MaybeUninit::uninit().assume_init() },
            head: 0,
            tail: 0,
            is_full: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head == self.tail && !self.is_full
    }

    pub fn is_full(&self) -> bool {
        self.is_full
    }

    pub fn len(&self) -> usize {
        let head = self.head;
        let tail = self.tail;

        if self.is_full() {
            N
        } else if tail >= head {
            tail - head
        } else {
            N - head + tail
        }
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

    fn wrap_add(idx: usize, addend: usize) -> usize {
        let (index, overflow) = idx.overflowing_add(addend);
        if index >= N || overflow {
            index.wrapping_sub(N)
        } else {
            index
        }
    }

    pub fn front_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            None
        } else {
            Some(unsafe { self.get_unchecked_mut(self.head) })
        }
    }

    pub fn clear(&mut self) {
        // SAFETY:
        // head tail track the initialized items and only drop the occupied slots with initialized
        // value.
        if self.tail >= self.head && !self.is_full {
            for t in &mut self.inner[self.head..self.tail] {
                unsafe { t.assume_init_drop() };
            }
        } else {
            for t in &mut self.inner[self.head..] {
                unsafe { t.assume_init_drop() };
            }

            for t in &mut self.inner[..self.tail] {
                unsafe { t.assume_init_drop() };
            }
        }

        self.tail = 0;
        self.head = 0;
        self.is_full = false;
    }

    pub fn pop_front(&mut self) -> Option<T> {
        if self.is_empty() {
            None
        } else {
            let head = self.head;
            self.head = Self::wrap_add(self.head, 1);
            if self.is_full {
                self.is_full = false;
            }
            unsafe { Some(self.read(head)) }
        }
    }

    pub fn push_back(&mut self, value: T) -> Result<(), PushError> {
        if self.is_full() {
            return Err(PushError);
        }

        self.is_full = self.len() == N - 1;

        unsafe { self.write(self.tail, value) };

        self.tail = Self::wrap_add(self.tail, 1);

        Ok(())
    }

    pub fn iter(&self) -> Iter<'_, T, N> {
        Iter {
            queue: self,
            head: self.head,
            len: self.len(),
        }
    }
}

pub struct PushError;

impl fmt::Debug for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PushError")
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
    head: usize,
    len: usize,
}

impl<'a, T, const N: usize> Iterator for Iter<'a, T, N> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        if self.len == 0 {
            return None;
        }

        let head = self.head;

        self.head = ArrayQueue::<T, N>::wrap_add(self.head, 1);

        self.len -= 1;

        unsafe { Some(self.queue.get_unchecked(head)) }
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

        queue.push_back(996).ok().unwrap();
        queue.push_back(231).ok().unwrap();
        queue.push_back(007).ok().unwrap();

        let mut iter = queue.iter();

        assert_eq!(iter.next(), Some(&996));
        assert_eq!(iter.next(), Some(&231));
        assert_eq!(iter.next(), Some(&007));
        assert_eq!(iter.next(), None);

        assert_eq!(queue.pop_front(), Some(996));

        let mut iter = queue.iter();

        assert_eq!(iter.next(), Some(&231));
        assert_eq!(iter.next(), Some(&007));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn cap() {
        let mut queue = ArrayQueue::<_, 3>::new();

        queue.push_back(996).ok().unwrap();
        queue.push_back(231).ok().unwrap();
        queue.push_back(007).ok().unwrap();

        assert!(queue.push_back(123).is_err());
    }

    #[test]
    fn front_mut() {
        let mut queue = ArrayQueue::<_, 3>::new();

        assert_eq!(None, queue.front_mut());

        queue.push_back(996).ok().unwrap();
        queue.push_back(231).ok().unwrap();
        queue.push_back(007).ok().unwrap();

        assert_eq!(Some(&mut 996), queue.front_mut());

        queue.pop_front();
        assert_eq!(Some(&mut 231), queue.front_mut());

        queue.pop_front();
        assert_eq!(Some(&mut 007), queue.front_mut());

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
        use std::sync::Arc;

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
