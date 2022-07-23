use std::io;

use bytes::{BufMut, Bytes, BytesMut};

pub struct Writer {
    buf: BytesMut,
}

impl Writer {
    pub(super) fn new() -> Writer {
        Writer { buf: BytesMut::new() }
    }

    pub(super) fn take(&mut self) -> Bytes {
        self.buf.split().freeze()
    }
}

impl io::Write for Writer {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.put_slice(buf);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
