use core::convert::Infallible;

use std::io::Write;

use crate::{
    bytes::{Buf, BufMut, BufMutWriter, Bytes, BytesMut, EitherBuf, buf::Chain},
    util::buffered::{BufWrite, ListWriteBuf, WriteBuf},
};

/// trait for add http/1 data to buffer that implement [BufWrite] trait.
pub trait H1BufWrite: BufWrite {
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
}

impl H1BufWrite for BytesMut {}

impl<const BUF_LIMIT: usize> H1BufWrite for WriteBuf<BUF_LIMIT> {}

// as special type for eof chunk when using transfer-encoding: chunked
type Eof = Chain<Chain<Bytes, Bytes>, &'static [u8]>;

type EncodedBuf<B, B2> = EitherBuf<B, EitherBuf<B2, &'static [u8]>>;

impl<const BUF_LIMIT: usize> H1BufWrite for ListWriteBuf<EncodedBuf<Bytes, Eof>, BUF_LIMIT> {
    fn write_buf_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        // list buffer use BufWrite::write_buf for temporary response head storage.
        // after the head is successfully written we must move the bytes to list.
        self.write_buf(func).inspect(|_| {
            let bytes = self.split_buf().freeze();
            self.buffer(EitherBuf::Left(bytes));
        })
    }

    #[inline]
    fn write_buf_static(&mut self, bytes: &'static [u8]) {
        self.buffer(EitherBuf::Right(EitherBuf::Right(bytes)));
    }

    #[inline]
    fn write_buf_bytes(&mut self, bytes: Bytes) {
        self.buffer(EitherBuf::Left(bytes));
    }

    #[inline]
    fn write_buf_bytes_chunked(&mut self, bytes: Bytes) {
        let chunk = Bytes::from(format!("{:X}\r\n", bytes.len()))
            .chain(bytes)
            .chain(b"\r\n" as &'static [u8]);
        self.buffer(EitherBuf::Right(EitherBuf::Left(chunk)));
    }
}

impl<L, R> H1BufWrite for EitherBuf<L, R>
where
    L: H1BufWrite,
    R: H1BufWrite,
{
    #[inline]
    fn write_buf_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        match *self {
            Self::Left(ref mut l) => l.write_buf_head(func),
            Self::Right(ref mut r) => r.write_buf_head(func),
        }
    }

    #[inline]
    fn write_buf_static(&mut self, bytes: &'static [u8]) {
        match *self {
            Self::Left(ref mut l) => l.write_buf_static(bytes),
            Self::Right(ref mut r) => r.write_buf_static(bytes),
        }
    }

    #[inline]
    fn write_buf_bytes(&mut self, bytes: Bytes) {
        match *self {
            Self::Left(ref mut l) => l.write_buf_bytes(bytes),
            Self::Right(ref mut r) => r.write_buf_bytes(bytes),
        }
    }

    #[inline]
    fn write_buf_bytes_chunked(&mut self, bytes: Bytes) {
        match *self {
            Self::Left(ref mut l) => l.write_buf_bytes_chunked(bytes),
            Self::Right(ref mut r) => r.write_buf_bytes_chunked(bytes),
        }
    }
}
