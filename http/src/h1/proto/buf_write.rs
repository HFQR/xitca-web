use core::convert::Infallible;

use std::io::Write;

use crate::bytes::{BufMut, BufMutWriter, Bytes, BytesMut};

/// trait for add http/1 data to buffer that implement [BufWrite] trait.
pub trait H1BufWrite {
    /// write http response head(status code and reason line, header lines) to buffer with fallible
    /// closure. on error path the buffer is reverted back to state before method was called.
    #[inline]
    fn write_buf_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        self.write_buf(func)
    }

    /// write `&'static [u8]` to buffer.
    fn write_buf_static(&mut self, bytes: &'static [u8]) {
        let _ = self.write_buf(|buf| {
            buf.put_slice(bytes);
            Ok::<_, Infallible>(())
        });
    }

    /// write bytes to buffer as is.
    fn write_buf_bytes(&mut self, bytes: Bytes) {
        let _ = self.write_buf(|buf| {
            buf.put_slice(bytes.as_ref());
            Ok::<_, Infallible>(())
        });
    }

    /// write bytes to buffer as `transfer-encoding: chunked` encoded.
    fn write_buf_bytes_chunked(&mut self, bytes: Bytes) {
        let _ = self.write_buf(|buf| {
            write!(BufMutWriter(buf), "{:X}\r\n", bytes.len()).unwrap();
            buf.reserve(bytes.len() + 2);
            buf.put_slice(bytes.as_ref());
            buf.put_slice(b"\r\n");
            Ok::<_, Infallible>(())
        });
    }

    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>;
}

impl H1BufWrite for BytesMut {
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        let len = self.len();
        func(self).inspect_err(|_| self.truncate(len))
    }
}
