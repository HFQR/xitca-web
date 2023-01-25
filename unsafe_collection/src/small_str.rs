extern crate alloc;

use core::{
    fmt,
    hash::{Hash, Hasher},
    mem::MaybeUninit,
    str,
};

use alloc::boxed::Box;

use super::uninit::{slice_assume_init, uninit_array, PartialInit};

/// Data structure aiming to have the same memory size of Box<str> that being able to store bytes
/// on stack and only allocate on heap only when necessary.
#[derive(Clone)]
pub enum SmallBoxedStr {
    Inline([MaybeUninit<u8>; 7], u8),
    Heap(Box<str>),
}

impl SmallBoxedStr {
    #[inline]
    pub const fn new() -> Self {
        Self::Inline(uninit_array(), 0)
    }

    pub fn as_str(&self) -> &str {
        match *self {
            Self::Inline(ref arr, len) => {
                let s = &arr[..len as usize];
                // SAFETY:
                // length and str validation are guaranteed when constructing SmallBoxedStr.
                unsafe { str::from_utf8_unchecked(slice_assume_init(s)) }
            }
            Self::Heap(ref boxed) => boxed,
        }
    }

    fn from_str(str: &str) -> Self {
        let len = str.len();
        if len > 7 {
            Self::Heap(Box::from(str))
        } else {
            let mut array = uninit_array();
            let slice = array.init_from(str.as_bytes().iter()).into_init_with(|f| *f);
            debug_assert_eq!(slice.len(), len);
            Self::Inline(array, len as u8)
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

#[cfg(test)]
mod test {
    use core::mem;

    use super::*;

    #[test]
    fn size() {
        assert_eq!(mem::size_of::<SmallBoxedStr>(), mem::size_of::<Box<str>>());
    }

    #[test]
    fn from_str() {
        let s = SmallBoxedStr::from("1234567");
        assert_eq!(s.as_str(), "1234567");
        assert!(matches!(s, SmallBoxedStr::Inline(_, _)));

        let s = SmallBoxedStr::from_str("12345678");
        assert_eq!(s.as_str(), "12345678");
        assert!(matches!(s, SmallBoxedStr::Heap(_)));
    }
}
