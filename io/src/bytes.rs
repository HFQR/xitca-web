//! re-export of [bytes] crate types.

pub use bytes::*;

use core::fmt;

use std::io;

/// A new type for help implementing [io::Write] and [fmt::Write] traits.
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

/// trait generic over different types of buffer strategy.
pub trait BufInterest {
    /// flag if buffer want more data to be filled in.
    fn want_write_buf(&self) -> bool;

    /// flag if buffer want to write data to io.
    fn want_write_io(&self) -> bool;
}

/// trait generic over different types of write buffer strategy.
pub trait BufWrite: BufInterest {
    /// write into [BytesMut] with closure that output a Result type.
    /// the result type is used to hint buffer to stop wanting to flush IO on [BufWrite::write_io]
    /// or revert BytesMut to previous state before method was called.
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>;

    /// write into IO from buffer.
    fn write_io<Io: io::Write>(&mut self, io: &mut Io) -> io::Result<()>;
}

impl BufInterest for BytesMut {
    #[inline]
    fn want_write_buf(&self) -> bool {
        true
    }

    #[inline]
    fn want_write_io(&self) -> bool {
        !self.is_empty()
    }
}

impl BufWrite for BytesMut {
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut Self) -> Result<T, E>,
    {
        let len = self.len();
        func(self).map_err(|e| {
            self.truncate(len);
            e
        })
    }

    fn write_io<Io: io::Write>(&mut self, _: &mut Io) -> io::Result<()> {
        unimplemented!("<BytesMut as BufWrite>::write_io is not implemented.")
    }
}

pub struct WriteBuf {
    buf: BytesMut,
    want_flush: bool,
}

impl WriteBuf {
    #[inline]
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            want_flush: false,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// clear remaining bytes in buffer and set flush flag to false.
    /// this would make following [BufInterest::want_write_io] call return false.
    #[inline]
    pub fn clear(&mut self) {
        self.buf.clear();
        self.want_flush = false;
    }

    #[inline]
    pub fn buf(&self) -> &[u8] {
        &self.buf
    }
}

impl Default for WriteBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl BufInterest for WriteBuf {
    #[inline]
    fn want_write_buf(&self) -> bool {
        self.buf.want_write_buf()
    }

    #[inline]
    fn want_write_io(&self) -> bool {
        self.buf.want_write_io() || self.want_flush
    }
}

impl BufWrite for WriteBuf {
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        self.buf.write_buf(func).map(|t| {
            self.want_flush = false;
            t
        })
    }

    fn write_io<Io: io::Write>(&mut self, io: &mut Io) -> io::Result<()> {
        loop {
            if self.want_flush {
                match io::Write::flush(io) {
                    Ok(_) => self.want_flush = false,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                break;
            }
            match io::Write::write(io, &self.buf) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.buf.advance(n);
                    if self.buf.is_empty() {
                        self.want_flush = true;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
