//! lazy type extractor.

use core::marker::PhantomData;

use std::borrow::Cow;

// lazy deserialize type that wrap around other extractor type like `Json`, `Form` and `Query`
pub struct Lazy<'a, T> {
    inner: Cow<'a, [u8]>,
    _type: PhantomData<T>,
}

impl<T> Lazy<'_, T> {
    pub(super) fn as_slice(&self) -> &[u8] {
        match self.inner {
            Cow::Owned(ref vec) => vec.as_slice(),
            Cow::Borrowed(slice) => slice,
        }
    }
}

impl<T> From<Vec<u8>> for Lazy<'static, T> {
    fn from(vec: Vec<u8>) -> Self {
        Self {
            inner: Cow::Owned(vec),
            _type: PhantomData,
        }
    }
}

impl<'a, T> From<&'a [u8]> for Lazy<'a, T> {
    fn from(slice: &'a [u8]) -> Self {
        Self {
            inner: Cow::Borrowed(slice),
            _type: PhantomData,
        }
    }
}
