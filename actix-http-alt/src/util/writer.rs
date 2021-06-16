use std::io::{self, Write};

use bytes::BufMut;

pub(crate) struct Writer<'a, B>(&'a mut B);

impl<'a, B> Writer<'a, B> {
    pub(crate) fn new(buf: &'a mut B) -> Self {
        Self(buf)
    }
}

impl<B> Write for Writer<'_, B>
where
    B: BufMut,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.put_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
