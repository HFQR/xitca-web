use std::{fmt, mem::MaybeUninit, ptr, slice};

pub struct ArrayQueue<T, const N: usize> {
    inner: MaybeUninit<[T; N]>,
    tail: usize,
    head: usize,
    is_full: bool,
}

impl<T, const N: usize> ArrayQueue<T, N> {
    pub const fn new() -> Self {
        ArrayQueue {
            inner: MaybeUninit::uninit(),
            tail: 0,
            head: 0,
            is_full: false,
        }
    }

    fn ptr(&self) -> *mut T {
        self.inner.as_ptr() as *mut T
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

    unsafe fn buffer_read(&mut self, off: usize) -> T {
        ptr::read(self.ptr().add(off))
    }

    unsafe fn buffer_write(&mut self, off: usize, value: T) {
        ptr::write(self.ptr().add(off), value);
    }

    fn wrap_add(idx: usize, addend: usize) -> usize {
        let (index, overflow) = idx.overflowing_add(addend);
        if index >= N || overflow {
            index.wrapping_sub(N)
        } else {
            index
        }
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.len() {
            let idx = Self::wrap_add(self.head, index);
            unsafe { Some(&*self.ptr().add(idx)) }
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len() {
            let idx = Self::wrap_add(self.head, index);
            unsafe { Some(&mut *self.ptr().add(idx)) }
        } else {
            None
        }
    }

    fn as_mut_slices(&mut self) -> (&mut [T], &mut [T]) {
        let ptr = self.ptr();
        if self.tail >= self.head && !self.is_full {
            (
                unsafe { slice::from_raw_parts_mut(ptr.add(self.head), self.tail - self.head) },
                &mut [],
            )
        } else {
            (
                unsafe { slice::from_raw_parts_mut(ptr.add(self.head), N - self.head) },
                unsafe { slice::from_raw_parts_mut(ptr, self.tail) },
            )
        }
    }

    pub fn clear(&mut self) {
        let (a, b) = self.as_mut_slices();
        unsafe { ptr::drop_in_place(a) };
        unsafe { ptr::drop_in_place(b) };
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
            unsafe { Some(self.buffer_read(head)) }
        }
    }

    pub fn push_back(&mut self, value: T) -> Result<(), PushError> {
        if self.is_full() {
            return Err(PushError);
        }

        self.is_full = self.len() == N - 1;

        unsafe { self.buffer_write(self.tail, value) };

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

        unsafe { Some(&*self.queue.ptr().add(head)) }
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
    fn drop() {
        use std::sync::Arc;

        let item = Arc::new(123);

        {
            let mut queue = ArrayQueue::<_, 3>::new();

            queue.push_back(item.clone()).ok().unwrap();

            assert_eq!(Arc::strong_count(&item), 2);
        }

        assert_eq!(Arc::strong_count(&item), 1);
    }
}
