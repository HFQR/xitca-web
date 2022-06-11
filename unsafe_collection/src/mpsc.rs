use core::{
    cell::UnsafeCell,
    slice,
    sync::atomic::{fence, AtomicUsize, Ordering},
};

use cache_padded::CachePadded;

use super::uninit::{self, UninitArray};

pub struct AtomicArray<T, const N: usize> {
    inner: UnsafeCell<UninitArray<T, N>>,
    next: CachePadded<AtomicUsize>,
    len: CachePadded<AtomicUsize>,
}

impl<T, const N: usize> AtomicArray<T, N> {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(UninitArray::new()),
            next: CachePadded::new(AtomicUsize::new(0)),
            len: CachePadded::new(AtomicUsize::new(0)),
        }
    }

    pub fn push_back(&self, value: T) -> Result<(), T> {
        let mut len = self.len.load(Ordering::Relaxed);

        loop {
            if len == N {
                return Err(value);
            }

            match self
                .len
                .compare_exchange_weak(len, len + 1, Ordering::AcqRel, Ordering::Relaxed)
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

    pub fn with_slice<F, O>(&self, func: F) -> O
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

    pub fn advance_by<F>(&self, mut func: F)
    where
        F: FnMut(&mut T) -> bool,
    {
        loop {
            let len = self.len.load(Ordering::Acquire);
            let idx = self.next.load(Ordering::Acquire);

            let tail = idx % N;

            let head = if tail >= len { tail - len } else { N + tail - len };

            // SAFETY:
            //
            // idx and len must correctly check the initialized items and their index.
            unsafe {
                let mut value = self.get_inner_mut().read_unchecked(head);

                if func(&mut value) {
                    self.len.fetch_sub(1, Ordering::Release);
                } else {
                    self.get_inner_mut().write_unchecked(head, value);
                    fence(Ordering::Release);
                    return;
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

        array.advance_by(|i| *i != 3);

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
}
