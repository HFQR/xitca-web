use crate::{
    body::SizeHint,
    http::header::{CONTENT_LENGTH, HeaderMap},
};

use super::frame::headers;

pub(crate) trait BodySize {
    /// Parse content-length from headers.
    /// Returns `Err(())` if the header is present but malformed (RFC 7540 §8.1.2.6).
    fn from_header(headers: &HeaderMap, is_end_stream: bool) -> Result<Self, ()>
    where
        Self: Sized;

    /// Subtract `len` bytes received in a DATA frame.
    /// Returns `Err(())` if this would underflow (overflow: more data than declared).
    fn dec(&mut self, len: usize) -> Result<(), ()>;

    /// Returns `Err` if remaining != 0 at END_STREAM (underflow: less data than declared).
    fn ensure_zero(&self) -> Result<(), ()>;
}

impl BodySize for SizeHint {
    fn from_header(headers: &HeaderMap, is_end_stream: bool) -> Result<Self, ()> {
        let res = match headers.get(CONTENT_LENGTH) {
            Some(v) => {
                let s = v.to_str().map_err(|_| ())?;
                match headers::parse_u64(s.as_bytes())? {
                    0 if is_end_stream => Self::None,
                    _ if is_end_stream => return Err(()),
                    n => Self::Exact(n),
                }
            }
            None if is_end_stream => Self::None,
            None => Self::Unknown,
        };
        Ok(res)
    }

    /// Subtract `len` bytes received in a DATA frame.
    /// Returns `Err(())` if this would underflow (overflow: more data than declared).
    fn dec(&mut self, len: usize) -> Result<(), ()> {
        match self {
            Self::Unknown => Ok(()),
            Self::None => Err(()),
            Self::Exact(rem) => {
                *rem = rem.checked_sub(len as u64).ok_or(())?;
                Ok(())
            }
        }
    }

    /// Returns `Err` if remaining != 0 at END_STREAM (underflow: less data than declared).
    fn ensure_zero(&self) -> Result<(), ()> {
        match self {
            Self::Unknown | Self::None | Self::Exact(0) => Ok(()),
            Self::Exact(_) => Err(()),
        }
    }
}
