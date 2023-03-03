use std::io::Write;

use xitca_io::bytes::{buf::Chain, Buf, BufMut, BufMutWriter, Bytes, BytesMut};
use xitca_unsafe_collection::bytes::EitherBuf;

use crate::util::buffered::{BufWrite, ListWriteBuf, WriteBuf};

/// trait for add http/1 data to buffer that implement [BufWrite] trait.
pub trait H1BufWrite: BufWrite {
    /// write response head bytes to buffer.
    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>;

    /// write static `&[u8]` to buffer.
    fn write_static(&mut self, bytes: &'static [u8]);

    /// write bytes to buffer as is.
    fn write_bytes(&mut self, bytes: Bytes);

    /// write bytes to buffer as `transfer-encoding: chunked` encoded.
    fn write_chunked(&mut self, bytes: Bytes);
}

impl<const BUF_LIMIT: usize> H1BufWrite for WriteBuf<BUF_LIMIT> {
    #[inline(always)]
    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        self.buf.write_head(func)
    }

    #[inline(always)]
    fn write_static(&mut self, bytes: &'static [u8]) {
        self.buf.write_static(bytes)
    }

    #[inline(always)]
    fn write_bytes(&mut self, bytes: Bytes) {
        self.buf.write_bytes(bytes)
    }

    #[inline(always)]
    fn write_chunked(&mut self, bytes: Bytes) {
        self.buf.write_chunked(bytes)
    }
}

impl H1BufWrite for BytesMut {
    #[inline]
    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        func(self)
    }

    #[inline]
    fn write_static(&mut self, bytes: &'static [u8]) {
        self.put_slice(bytes);
    }

    #[inline]
    fn write_bytes(&mut self, bytes: Bytes) {
        self.put_slice(bytes.as_ref());
    }

    fn write_chunked(&mut self, bytes: Bytes) {
        write!(BufMutWriter(self), "{:X}\r\n", bytes.len()).unwrap();
        self.reserve(bytes.len() + 2);
        self.put_slice(bytes.as_ref());
        self.put_slice(b"\r\n");
    }
}

// as special type for eof chunk when using transfer-encoding: chunked
type Eof = Chain<Chain<Bytes, Bytes>, &'static [u8]>;

type EncodedBuf<B, B2> = EitherBuf<B, EitherBuf<B2, &'static [u8]>>;

impl<const BUF_LIMIT: usize> H1BufWrite for ListWriteBuf<EncodedBuf<Bytes, Eof>, BUF_LIMIT> {
    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        let buf = &mut self.buf;
        let res = func(buf)?;
        let bytes = buf.split().freeze();
        self.buffer(EitherBuf::Left(bytes));
        Ok(res)
    }

    #[inline]
    fn write_static(&mut self, bytes: &'static [u8]) {
        self.buffer(EitherBuf::Right(EitherBuf::Right(bytes)));
    }

    #[inline]
    fn write_bytes(&mut self, bytes: Bytes) {
        self.buffer(EitherBuf::Left(bytes));
    }

    #[inline]
    fn write_chunked(&mut self, bytes: Bytes) {
        let eof = Bytes::from(format!("{:X}\r\n", bytes.len()))
            .chain(bytes)
            .chain(b"\r\n" as &'static [u8]);
        self.buffer(EitherBuf::Right(EitherBuf::Left(eof)));
    }
}
