//! re-export of [bytes] crate types.

pub use bytes::*;

use core::fmt;

use std::io;

/// A new type for help implementing [std::io::Write] and [std::fmt::Write] traits.
pub struct BufMutWriter<'a, B>(pub &'a mut B);

impl<B: BufMut> io::Write for BufMutWriter<'_, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.put_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<B: BufMut> fmt::Write for BufMutWriter<'_, B> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.put_slice(s.as_bytes());
        Ok(())
    }
}
