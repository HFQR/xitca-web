use core::convert::Infallible;

use std::io::Write;

use xitca_io::bytes::{buf::Chain, Buf, BufMut, BufMutWriter, Bytes, BytesMut};
use xitca_unsafe_collection::bytes::EitherBuf;

use crate::util::buffered::{BufWrite, ListWriteBuf, WriteBuf};

/// trait for add http/1 data to buffer that implement [BufWrite] trait.
pub trait H1BufWrite: BufWrite {
    /// write to buffer with fallible closure.
    /// on error path the buffer would be revert to state before the call happen.
    fn write_buf_fallible<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>;

    /// write `&'static [u8]` to buffer.
    fn write_buf_static(&mut self, bytes: &'static [u8]);

    /// write bytes to buffer as is.
    fn write_buf_bytes(&mut self, bytes: Bytes);

    /// write bytes to buffer as `transfer-encoding: chunked` encoded.
    fn write_buf_bytes_chunked(&mut self, bytes: Bytes);
}

impl<const BUF_LIMIT: usize> H1BufWrite for WriteBuf<BUF_LIMIT> {
    #[inline(always)]
    fn write_buf_fallible<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        self.buf.write_buf_fallible(func)
    }

    #[inline(always)]
    fn write_buf_static(&mut self, bytes: &'static [u8]) {
        self.buf.write_buf_static(bytes)
    }

    #[inline(always)]
    fn write_buf_bytes(&mut self, bytes: Bytes) {
        self.buf.write_buf_bytes(bytes)
    }

    #[inline(always)]
    fn write_buf_bytes_chunked(&mut self, bytes: Bytes) {
        self.buf.write_buf_bytes_chunked(bytes)
    }
}

impl H1BufWrite for BytesMut {
    #[inline]
    fn write_buf_fallible<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        let len = self.len();
        self.write_buf(func).map_err(|e| {
            self.truncate(len);
            e
        })
    }

    #[inline]
    fn write_buf_static(&mut self, bytes: &'static [u8]) {
        let _ = self.write_buf(|this| Ok::<_, Infallible>(this.put_slice(bytes)));
    }

    #[inline]
    fn write_buf_bytes(&mut self, bytes: Bytes) {
        let _ = self.write_buf(|this| Ok::<_, Infallible>(this.put_slice(bytes.as_ref())));
    }

    fn write_buf_bytes_chunked(&mut self, bytes: Bytes) {
        let _ = self.write_buf(|this| {
            write!(BufMutWriter(this), "{:X}\r\n", bytes.len()).unwrap();
            this.reserve(bytes.len() + 2);
            this.put_slice(bytes.as_ref());
            this.put_slice(b"\r\n");
            Ok::<_, Infallible>(())
        });
    }
}

// as special type for eof chunk when using transfer-encoding: chunked
type Eof = Chain<Chain<Bytes, Bytes>, &'static [u8]>;

type EncodedBuf<B, B2> = EitherBuf<B, EitherBuf<B2, &'static [u8]>>;

impl<const BUF_LIMIT: usize> H1BufWrite for ListWriteBuf<EncodedBuf<Bytes, Eof>, BUF_LIMIT> {
    fn write_buf_fallible<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        self.write_buf(|this| {
            let buf = &mut this.buf;
            match func(buf) {
                Ok(t) => {
                    let bytes = buf.split().freeze();
                    this.buffer(EitherBuf::Left(bytes));
                    Ok(t)
                }
                Err(e) => {
                    buf.clear();
                    Err(e)
                }
            }
        })
    }

    #[inline]
    fn write_buf_static(&mut self, bytes: &'static [u8]) {
        let _ = self.write_buf(|this| Ok::<_, Infallible>(this.buffer(EitherBuf::Right(EitherBuf::Right(bytes)))));
    }

    #[inline]
    fn write_buf_bytes(&mut self, bytes: Bytes) {
        let _ = self.write_buf(|this| Ok::<_, Infallible>(this.buffer(EitherBuf::Left(bytes))));
    }

    #[inline]
    fn write_buf_bytes_chunked(&mut self, bytes: Bytes) {
        let _ = self.write_buf(|this| {
            let eof = Bytes::from(format!("{:X}\r\n", bytes.len()))
                .chain(bytes)
                .chain(b"\r\n" as &'static [u8]);
            this.buffer(EitherBuf::Right(EitherBuf::Left(eof)));
            Ok::<_, Infallible>(())
        });
    }
}
