use core::{
    fmt,
    hash::{Hash, Hasher},
    str,
};

use inner::Inner;

mod inner {
    extern crate alloc;

    use core::{
        mem::{ManuallyDrop, MaybeUninit},
        ptr::{self, NonNull},
        slice,
    };

    use crate::uninit::slice_assume_init;

    const TAG: u8 = 0b1000_0000;

    pub(super) union Inner {
        inline: Inline,
        heap: ManuallyDrop<Heap>,
    }

    #[derive(Copy, Clone)]
    struct Inline {
        arr: [MaybeUninit<u8>; 15],
        len: u8,
    }

    impl Inline {
        // SAFETY:
        // caller must make sure slice is no more than 15 bytes in length.
        unsafe fn from_slice(slice: &[u8]) -> Self {
            let mut arr = [const { MaybeUninit::uninit() }; 15];

            let len = slice.len();

            unsafe {
                ptr::copy_nonoverlapping(slice.as_ptr(), arr.as_mut_ptr().cast(), len);
            }
            let len = TAG | (len as u8);

            Inline { arr, len }
        }

        // SAFETY:
        // caller must make sure the variant is properly intialized.
        unsafe fn as_slice(&self) -> &[u8] {
            let len = (self.len & !TAG) as usize;
            let s = &self.arr[..len];
            unsafe { slice_assume_init(s) }
        }
    }

    struct Heap {
        ptr: NonNull<u8>,
        len: usize,
    }

    impl Heap {
        fn clone(&self) -> ManuallyDrop<Self> {
            // SAFETY:
            // Heap is intialized before cloning
            let slice = unsafe { self.as_slice() };
            Self::from_slice(slice)
        }

        fn from_slice(slice: &[u8]) -> ManuallyDrop<Self> {
            let len = slice.len();

            // the last u8's highest bit is used as union tag of Inner.
            if len > isize::MAX as usize {
                panic!("capacity overflow");
            }

            let boxed = Box::<[u8]>::from(slice);
            let ptr = Box::into_raw(boxed).cast();

            // SAFETY:
            // ptr is from newly allcolated box and it's not null.
            let heap = unsafe { NonNull::new_unchecked(ptr) };
            ManuallyDrop::new(Heap { ptr: heap, len })
        }

        // SAFETY:
        // caller must make sure the variant is properly intialized.
        const unsafe fn as_slice(&self) -> &[u8] {
            // see Inner::empty method for reason.
            #[cfg(target_endian = "big")]
            {
                if self.len == 0 {
                    return &[];
                }
            }

            let ptr = self.ptr.as_ptr();
            unsafe { slice::from_raw_parts(ptr, self.len) }
        }
    }

    impl Inner {
        pub(super) const fn empty() -> Self {
            #[cfg(target_endian = "little")]
            {
                Self {
                    inline: Inline {
                        arr: [const { MaybeUninit::uninit() }; 15],
                        len: TAG,
                    },
                }
            }

            // for bigendian default to heap variant. the pointer is dangling and
            // Heap::as_slice method must check the length to avoid dereferencing it.
            #[cfg(target_endian = "big")]
            {
                Self {
                    heap: Heap {
                        ptr: NonNull::dangling(),
                        len: 0,
                    },
                }
            }
        }

        pub(super) const fn is_inline(&self) -> bool {
            #[cfg(target_endian = "little")]
            {
                // SAFETY:
                // for either Heap or Inline variant the tag u8 is always initliazed.
                let tag = unsafe { self.inline.len };
                tag & TAG == TAG
            }

            // for bigendian always use heap variant.
            #[cfg(target_endian = "big")]
            {
                false
            }
        }

        pub(super) fn from_slice(slice: &[u8]) -> Self {
            // for bigendian always use heap variant.
            #[cfg(target_endian = "little")]
            if slice.len() < 16 {
                return Self {
                    // SAFETY:
                    // slice is no more than 15 bytes.
                    inline: unsafe { Inline::from_slice(slice) },
                };
            }

            Self {
                heap: Heap::from_slice(slice),
            }
        }

        pub(super) fn as_slice(&self) -> &[u8] {
            if self.is_inline() {
                // SAFETY:
                // just checked the variant is inline.
                unsafe { self.inline.as_slice() }
            } else {
                // SAFETY:
                // just checked the variant is Heap.
                unsafe { self.heap.as_slice() }
            }
        }
    }

    impl Clone for Inner {
        fn clone(&self) -> Self {
            if self.is_inline() {
                // SAFETY:
                // just checked the variant is inline.
                let inline = unsafe { self.inline };
                Inner { inline }
            } else {
                // SAFETY:
                // just checked the variant is Heap.
                let heap = unsafe { self.heap.clone() };
                Inner { heap }
            }
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            if !self.is_inline() {
                // SAFETY:
                // just check the variant is Heap.
                // manually drop the allocated boxed slice.
                unsafe {
                    let ptr = self.heap.ptr.as_ptr();
                    let len = self.heap.len;
                    let ptr = ptr::slice_from_raw_parts_mut(ptr, len);
                    let _ = Box::from_raw(ptr);
                }
            }
            // inline field is Copy which do not need explicit destruction.
        }
    }

    // SAFETY
    // it's safe to share Inner between threads.
    unsafe impl Send for Inner {}
    unsafe impl Sync for Inner {}
}

/// Data structure aiming to have the same memory size of `Box<str>` that being able to store str
/// on stack and only allocate on heap when necessary.
#[derive(Clone)]
pub struct SmallBoxedStr {
    inner: Inner,
}

impl SmallBoxedStr {
    #[inline]
    pub const fn new() -> Self {
        Self { inner: Inner::empty() }
    }

    fn as_str(&self) -> &str {
        let slice = self.inner.as_slice();

        // SAFETY
        // str validation is guaranteed when constructing.
        unsafe { str::from_utf8_unchecked(slice) }
    }

    fn from_str(str: &str) -> Self {
        Self {
            inner: Inner::from_slice(str.as_bytes()),
        }
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

fn _assert_send_sync<T: Send + Sync>() {}

#[cfg(test)]
mod test {
    use core::mem;

    use super::*;

    #[test]
    fn send_sync() {
        _assert_send_sync::<SmallBoxedStr>();
    }

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
        let s = SmallBoxedStr::from("123456789012345");
        assert!(s.inner.is_inline());
        assert_eq!(&s, "123456789012345");

        let s = SmallBoxedStr::from_str("1234567890123456");
        assert!(!s.inner.is_inline());
        assert_eq!(&s, "1234567890123456");
    }

    #[test]
    fn clone_and_drop() {
        let s = SmallBoxedStr::from_str("1234567890123456");
        assert_eq!(&s, "1234567890123456");

        let s1 = s.clone();
        assert_eq!(s, s1);
        drop(s);
        drop(s1);
    }
}
