//! Simple bounded ring buffers with FIFO queue.

pub mod heap;
pub mod stack;

use core::fmt;

pub trait Queueable {
    type Item;

    // capacity of Self for check bound
    fn capacity(&self) -> usize;

    /// # Safety
    /// caller must make sure given index is not out of bound and properly initialized.
    unsafe fn _get_unchecked(&self, idx: usize) -> &Self::Item;

    /// # Safety
    /// caller must make sure given index is not out of bound and properly initialized.
    unsafe fn _get_mut_unchecked(&mut self, idx: usize) -> &mut Self::Item;

    /// # Safety
    /// caller must make sure given index is not out of bound and properly initialized.
    unsafe fn _read_unchecked(&mut self, idx: usize) -> Self::Item;

    /// # Safety
    /// caller must make sure given index is not out of bound and properly initialized.
    unsafe fn _write_unchecked(&mut self, idx: usize, item: Self::Item);
}

struct BoundedQuery<Q>
where
    Q: Queueable,
{
    queue: Q,
    next: usize,
    len: usize,
}

impl<Q> BoundedQuery<Q>
where
    Q: Queueable,
{
    const fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn is_full(&self) -> bool {
        self.len == self.queue.capacity()
    }

    const fn len(&self) -> usize {
        self.len
    }

    fn incr_tail_len(&mut self) {
        self.next += 1;
        self.len += 1;

        if self.next == self.queue.capacity() {
            self.next = 0;
        }
    }

    fn front(&self) -> Option<&Q::Item> {
        if self.is_empty() {
            None
        } else {
            Some(unsafe { self.front_unchecked() })
        }
    }

    // SAFETY:
    // caller must make sure self is not empty
    unsafe fn front_unchecked(&self) -> &Q::Item {
        let idx = self.front_idx();
        self.queue._get_unchecked(idx)
    }

    fn front_mut(&mut self) -> Option<&mut Q::Item> {
        if self.is_empty() {
            None
        } else {
            Some(unsafe { self.front_mut_unchecked() })
        }
    }

    // SAFETY:
    // caller must make sure self is not empty
    unsafe fn front_mut_unchecked(&mut self) -> &mut Q::Item {
        let idx = self.front_idx();
        self.queue._get_mut_unchecked(idx)
    }

    fn clear(&mut self) {
        while self.pop_front().is_some() {}
        self.next = 0;
        self.len = 0;
    }

    fn pop_front(&mut self) -> Option<Q::Item> {
        if self.is_empty() {
            None
        } else {
            unsafe { Some(self.pop_front_unchecked()) }
        }
    }

    // SAFETY:
    // caller must make sure self is not empty
    unsafe fn pop_front_unchecked(&mut self) -> Q::Item {
        let idx = self.front_idx();
        self.len -= 1;
        self.queue._read_unchecked(idx)
    }

    fn push_back(&mut self, item: Q::Item) -> Result<(), PushError<Q::Item>> {
        if self.is_full() {
            Err(PushError(item))
        } else {
            unsafe {
                self.push_back_unchecked(item);
            }
            Ok(())
        }
    }

    // SAFETY:
    // caller must make sure self is not full.
    unsafe fn push_back_unchecked(&mut self, item: Q::Item) {
        self.queue._write_unchecked(self.next, item);
        self.incr_tail_len();
    }

    const fn iter(&self) -> Iter<'_, Q> {
        Iter {
            queue: &self.queue,
            tail: self.next,
            len: self.len(),
        }
    }

    fn front_idx(&self) -> usize {
        if self.next >= self.len {
            self.next - self.len
        } else {
            self.queue.capacity() + self.next - self.len
        }
    }
}

impl<Q> fmt::Debug for BoundedQuery<Q>
where
    Q: Queueable,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArrayQueue")
    }
}

impl<Q> Drop for BoundedQuery<Q>
where
    Q: Queueable,
{
    fn drop(&mut self) {
        self.clear();
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

#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
#[derive(Clone)]
pub struct Iter<'a, Q>
where
    Q: Queueable,
{
    queue: &'a Q,
    tail: usize,
    len: usize,
}

impl<'a, Q> Iterator for Iter<'a, Q>
where
    Q: Queueable,
{
    type Item = &'a Q::Item;

    #[inline]
    fn next(&mut self) -> Option<&'a Q::Item> {
        if self.len == 0 {
            return None;
        }

        let idx = if self.tail >= self.len {
            self.tail - self.len
        } else {
            self.queue.capacity() + self.tail - self.len
        };

        self.len -= 1;

        unsafe { Some(self.queue._get_unchecked(idx)) }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}
