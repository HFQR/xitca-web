use std::io;

use bytes::{Bytes, BytesMut};

pub struct BytesMutWriter(BytesMut);

impl BytesMutWriter {
    pub(super) fn new() -> Self {
        Self(BytesMut::new())
    }

    pub(super) fn take(&mut self) -> Bytes {
        self.0.split().freeze()
    }

    #[cfg(feature = "br")]
    pub(super) fn take_owned(self) -> Bytes {
        self.0.freeze()
    }
}

impl io::Write for BytesMutWriter {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
