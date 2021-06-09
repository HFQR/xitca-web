use std::io;

use bytes::{BufMut, Bytes, BytesMut};

pub(super) const MAX_CHUNK_SIZE_DECODE_IN_PLACE: usize = 2049;

pub(crate) struct Writer {
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
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.put_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
