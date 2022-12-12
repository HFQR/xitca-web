use core::{
    fmt,
    ops::{Deref, RangeBounds},
    str::{self, Utf8Error},
};

use bytes_crate::Bytes;

/// reference counted String type. cheap to Clone and share between multiple threads.
#[derive(Clone, Default, Debug, Eq, PartialEq, Hash)]
pub struct BytesStr(Bytes);

impl fmt::Display for BytesStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl BytesStr {
    #[inline]
    pub const fn new() -> Self {
        Self(Bytes::new())
    }

    /// Compile time static str to BytesStr conversion.
    #[inline]
    pub const fn from_static(value: &'static str) -> Self {
        BytesStr(Bytes::from_static(value.as_bytes()))
    }

    /// Returns a slice of self for the provided range.
    #[inline]
    pub fn slice(&self, range: impl RangeBounds<usize>) -> Self {
        Self(self.0.slice(range))
    }

    /// Get ownership of inner [Bytes] value.
    #[inline]
    pub fn into_inner(self) -> Bytes {
        self.0
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        // SAFETY: check valid utf-8 in constructor
        unsafe { str::from_utf8_unchecked(self.0.as_ref()) }
    }
}

impl From<&str> for BytesStr {
    fn from(value: &str) -> Self {
        BytesStr(Bytes::copy_from_slice(value.as_bytes()))
    }
}

impl TryFrom<Bytes> for BytesStr {
    type Error = Utf8Error;

    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        str::from_utf8(value.as_ref())?;
        Ok(BytesStr(value))
    }
}

impl TryFrom<&[u8]> for BytesStr {
    type Error = Utf8Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        str::from_utf8(value)?;
        Ok(BytesStr(Bytes::copy_from_slice(value)))
    }
}

impl Deref for BytesStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for BytesStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for BytesStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
