use core::mem::ManuallyDrop;

use super::{BoundedQuery, PushError, Queueable};

pub struct HeapQueue<T> {
    inner: BoundedQuery<HeapQueueInner<T>>,
}

impl<T> HeapQueue<T> {
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: BoundedQuery {
                queue: HeapQueueInner {
                    ptr: ManuallyDrop::new(Vec::with_capacity(cap)).as_mut_ptr(),
                    cap,
                },
                next: 0,
                len: 0,
            },
        }
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn front(&self) -> Option<&T> {
        self.inner.front()
    }

    #[inline]
    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.inner.front_mut()
    }

    #[inline]
    pub fn push_back(&mut self, item: T) -> Result<(), PushError<T>> {
        self.inner.push_back(item)
    }

    /// # Safety
    ///
    /// caller must make sure self is not full.
    #[inline]
    pub unsafe fn push_back_unchecked(&mut self, item: T) {
        self.inner.push_back_unchecked(item)
    }

    #[inline]
    pub fn pop_front(&mut self) -> Option<T> {
        self.inner.pop_front()
    }

    /// # Safety
    ///
    /// caller must make sure self is not empty.
    #[inline]
    pub unsafe fn pop_front_unchecked(&mut self) -> T {
        self.inner.pop_front_unchecked()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        self.inner.iter()
    }
}

pub type Iter<'a, T> = super::Iter<'a, HeapQueueInner<T>>;

#[doc(hidden)]
pub struct HeapQueueInner<T> {
    ptr: *mut T,
    cap: usize,
}

impl<T> Drop for HeapQueueInner<T> {
    fn drop(&mut self) {
        // SAFETY:
        // deallocate the pointer but don't drop anything.
        // *. BoundQueue is tasked with drop all remaining item.
        unsafe {
            Vec::from_raw_parts(self.ptr, 0, self.cap);
        }
    }
}

impl<T> Queueable for HeapQueueInner<T> {
    type Item = T;

    #[inline(always)]
    fn capacity(&self) -> usize {
        self.cap
    }

    // SAFETY: see trait definition.
    #[inline]
    unsafe fn _get_unchecked(&self, idx: usize) -> &Self::Item {
        &*self.ptr.add(idx)
    }

    // SAFETY: see trait definition.
    #[inline]
    unsafe fn _get_mut_unchecked(&mut self, idx: usize) -> &mut Self::Item {
        &mut *self.ptr.add(idx)
    }

    // SAFETY: see trait definition.
    #[inline]
    unsafe fn _read_unchecked(&mut self, idx: usize) -> Self::Item {
        self.ptr.add(idx).read()
    }

    // SAFETY: see trait definition.
    #[inline]
    unsafe fn _write_unchecked(&mut self, idx: usize, item: Self::Item) {
        self.ptr.add(idx).write(item)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn iterate() {
        let mut queue = HeapQueue::with_capacity(5);

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
        let mut queue = HeapQueue::with_capacity(3);

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
        let mut queue = HeapQueue::with_capacity(3);

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
        let mut queue = HeapQueue::with_capacity(4);

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
            let _queue = HeapQueue::<usize>::with_capacity(3);
        }

        {
            let mut queue = HeapQueue::with_capacity(3);

            queue.push_back(item.clone()).ok().unwrap();
            queue.push_back(item.clone()).ok().unwrap();

            assert_eq!(Arc::strong_count(&item), 3);
        }

        assert_eq!(Arc::strong_count(&item), 1);

        {
            let mut queue = HeapQueue::with_capacity(3);

            queue.push_back(item.clone()).ok().unwrap();

            assert_eq!(Arc::strong_count(&item), 2);
        }

        assert_eq!(Arc::strong_count(&item), 1);

        {
            let mut queue = HeapQueue::with_capacity(3);

            queue.push_back(item.clone()).ok().unwrap();
            queue.push_back(item.clone()).ok().unwrap();
            queue.push_back(item.clone()).ok().unwrap();

            assert_eq!(Arc::strong_count(&item), 4);
        }

        assert_eq!(Arc::strong_count(&item), 1);
    }
}
