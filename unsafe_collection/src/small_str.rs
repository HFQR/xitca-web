use core::{
    fmt,
    hash::{Hash, Hasher},
    str,
};

use inner::Inner;

mod inner {
    extern crate alloc;

    use core::{
        mem::MaybeUninit,
        ptr::{self, NonNull},
        slice,
    };

    use alloc::boxed::Box;

    use crate::uninit::{slice_assume_init, uninit_array};

    // union is used as it can stably fit 8 bytes inline while enum requires one byte for tag.
    pub(super) union Inner {
        inline: [MaybeUninit<u8>; 8],
        heap: NonNull<u8>,
    }

    impl Inner {
        // SAFETY:
        // given slice must not be empty.
        unsafe fn heap_from_slice(slice: &[u8]) -> Self {
            let boxed = Box::<[u8]>::from(slice);
            let ptr = Box::into_raw(boxed).cast();
            let heap = NonNull::new_unchecked(ptr);
            Self { heap }
        }

        pub(super) const fn empty() -> Self {
            Self { inline: uninit_array() }
        }

        pub(super) fn from_slice(slice: &[u8]) -> Self {
            let len = slice.len();
            if len > 8 {
                // SAFETY:
                // slice is not empty
                unsafe { Inner::heap_from_slice(slice) }
            } else {
                let mut inline = uninit_array();

                // SAFETY:
                // copy slice to inlined array.
                unsafe {
                    ptr::copy_nonoverlapping(slice.as_ptr(), inline.as_mut_ptr().cast(), len);
                }

                Inner { inline }
            }
        }

        // SAFETY:
        // caller must make sure len equals to the length of input &[u8] when constructing Self with
        // Inner::from_slice function.
        pub(super) unsafe fn as_slice(&self, len: usize) -> &[u8] {
            if len > 8 {
                let ptr = self.heap.as_ptr();
                slice::from_raw_parts(ptr, len)
            } else {
                let s = &self.inline[..len];
                slice_assume_init(s)
            }
        }

        // SAFETY:
        // caller must make sure len equals to the length of input &[u8] when constructing Self with
        // Inner::from_slice function.
        pub(super) unsafe fn clone_with(&self, len: usize) -> Self {
            if len > 8 {
                let ptr = self.heap.as_ptr();
                let slice = slice::from_raw_parts(ptr, len);
                Inner::heap_from_slice(slice)
            } else {
                let inline = self.inline;
                Inner { inline }
            }
        }

        // SAFETY:
        // caller must make sure len equals to the length of input &[u8] when constructing Self with
        // Inner::from_slice function.
        // caller must not call this method multiple times on a single Self instance.
        pub(super) unsafe fn drop_with(&mut self, len: usize) {
            if len > 8 {
                let ptr = self.heap.as_ptr();
                let ptr = ptr::slice_from_raw_parts_mut(ptr, len);
                drop(Box::from_raw(ptr));
            }

            // inline field is Copy which do not need explicit destruction.
        }
    }
}

/// Data structure aiming to have the same memory size of `Box<str>` that being able to store str
/// on stack and only allocate on heap when necessary.
pub struct SmallBoxedStr {
    len: usize,
    inner: Inner,
}

impl SmallBoxedStr {
    #[inline]
    pub const fn new() -> Self {
        Self {
            len: 0,
            inner: Inner::empty(),
        }
    }

    fn as_str(&self) -> &str {
        // SAFETY
        // len validation is guaranteed when constructing.
        let slice = unsafe { self.inner.as_slice(self.len) };

        // SAFETY
        // str validation is guaranteed when constructing.
        unsafe { str::from_utf8_unchecked(slice) }
    }

    fn from_str(str: &str) -> Self {
        Self {
            len: str.len(),
            inner: Inner::from_slice(str.as_bytes()),
        }
    }
}

// SAFETY
// it's safe to share SmallBoxedStr between threads.
unsafe impl Send for SmallBoxedStr {}
unsafe impl Sync for SmallBoxedStr {}

impl Clone for SmallBoxedStr {
    fn clone(&self) -> Self {
        Self {
            len: self.len,
            // SAFETY:
            // length is guaranteed when constructing.
            inner: unsafe { self.inner.clone_with(self.len) },
        }
    }
}

impl Drop for SmallBoxedStr {
    fn drop(&mut self) {
        // SAFETY:
        // length is guaranteed when constructing.
        unsafe { self.inner.drop_with(self.len) };
    }
}

impl From<&str> for SmallBoxedStr {
    fn from(value: &str) -> Self {
        Self::from_str(value)
    }
}

impl AsRef<str> for SmallBoxedStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for SmallBoxedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Eq for SmallBoxedStr {}

impl PartialEq for SmallBoxedStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Hash for SmallBoxedStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(self.as_str().as_bytes())
    }
}

impl PartialEq<str> for SmallBoxedStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

#[cfg(test)]
mod test {
    use core::mem;

    use super::*;

    #[test]
    fn size() {
        assert_eq!(mem::size_of::<SmallBoxedStr>(), 16);
    }

    #[test]
    fn empty() {
        let s = SmallBoxedStr::new();
        assert!(s.as_str().is_empty());
    }

    #[test]
    fn from_str() {
        let s = SmallBoxedStr::from("12345678");
        assert_eq!(&s, "12345678");

        let s = SmallBoxedStr::from_str("123456789");
        assert_eq!(&s, "123456789");
    }

    #[test]
    fn clone_and_drop() {
        let s = SmallBoxedStr::from_str("1234567890");
        assert_eq!(&s, "1234567890");

        let s1 = s.clone();
        assert_eq!(s, s1);
        drop(s);
        drop(s1);
    }
}
