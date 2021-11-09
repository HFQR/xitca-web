use std::io::{self, Write};

use crate::bytes::BufMut;

pub(crate) struct Writer<'a, B>(&'a mut B);

impl<'a, B> Writer<'a, B> {
    pub(crate) fn new(buf: &'a mut B) -> Self {
        Self(buf)
    }
}

impl<B: BufMut> Write for Writer<'_, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.put_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
