use std::{fmt, ops::Deref};

use bytes_crate::Bytes;

/// reference counted String type. cheap to Clone and share between multiple threads.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct BytesStr(Bytes);

impl BytesStr {
    pub const fn from_static(value: &'static str) -> Self {
        BytesStr(Bytes::from_static(value.as_bytes()))
    }

    pub fn try_from(bytes: Bytes) -> Result<Self, std::str::Utf8Error> {
        std::str::from_utf8(bytes.as_ref())?;
        Ok(BytesStr(bytes))
    }

    pub fn try_from_slice(slice: impl AsRef<[u8]>) -> Result<Self, std::str::Utf8Error> {
        let slice = slice.as_ref();
        std::str::from_utf8(slice)?;
        Ok(BytesStr(Bytes::copy_from_slice(slice)))
    }

    pub fn copy_from_str(value: &str) -> Self {
        BytesStr(Bytes::copy_from_slice(value.as_bytes()))
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: check valid utf-8 in constructor
        unsafe { std::str::from_utf8_unchecked(self.0.as_ref()) }
    }

    pub fn into_inner(self) -> Bytes {
        self.0
    }
}

impl Deref for BytesStr {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for BytesStr {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl AsRef<str> for BytesStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for BytesStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}
