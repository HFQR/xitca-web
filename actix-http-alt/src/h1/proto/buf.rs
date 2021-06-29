//! Copied from `hyper::proto::h1::io`.
//! A write buffer that use vectored buf list.

use std::{
    fmt,
    io::{self, Write},
    ops::{Deref, DerefMut},
};

use actix_server_alt::net::AsyncReadWrite;
use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::h1::error::Error;
use crate::util::{buf_list::BufList, writer::Writer};

pub(super) struct ReadBuf<const READ_BUF_LIMIT: usize> {
    buf: BytesMut,
}

impl<const READ_BUF_LIMIT: usize> ReadBuf<READ_BUF_LIMIT> {
    pub(super) fn new() -> Self {
        Self { buf: BytesMut::new() }
    }

    #[inline(always)]
    pub(super) fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline(always)]
    pub(super) fn backpressure(&self) -> bool {
        self.buf.len() >= READ_BUF_LIMIT
    }

    #[inline(always)]
    pub(super) fn buf_mut(&mut self) -> &mut BytesMut {
        &mut self.buf
    }
}

/// Trait to generic over different types of write buffer strategy.
pub(super) trait WriteBuf<const WRITE_BUF_LIMIT: usize> {
    #[inline(always)]
    fn backpressure(&self) -> bool {
        self.len() >= WRITE_BUF_LIMIT
    }

    #[inline(always)]
    fn empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize;

    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>;

    fn write_static(&mut self, bytes: &'static [u8]);

    fn write_buf(&mut self, bytes: Bytes);

    fn write_eof(&mut self, bytes: Bytes);

    fn try_write_io<Io: AsyncReadWrite>(&mut self, io: &mut Io) -> Result<bool, Error>;
}

pub(super) struct FlatWriteBuf(BytesMut);

impl Default for FlatWriteBuf {
    fn default() -> Self {
        Self(BytesMut::new())
    }
}

impl Deref for FlatWriteBuf {
    type Target = BytesMut;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FlatWriteBuf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<const WRITE_BUF_LIMIT: usize> WriteBuf<WRITE_BUF_LIMIT> for FlatWriteBuf {
    fn len(&self) -> usize {
        self.remaining()
    }

    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        func(&mut *self)
    }

    fn write_static(&mut self, bytes: &'static [u8]) {
        self.put_slice(bytes);
    }

    fn write_buf(&mut self, bytes: Bytes) {
        self.put_slice(bytes.as_ref());
    }

    fn write_eof(&mut self, bytes: Bytes) {
        write!(Writer::new(&mut **self), "{:X}\r\n", bytes.len()).unwrap();

        self.reserve(bytes.len() + 2);
        self.put_slice(bytes.as_ref());
        self.put_slice(b"\r\n");
    }

    fn try_write_io<Io: AsyncReadWrite>(&mut self, io: &mut Io) -> Result<bool, Error> {
        let mut written = 0;
        let len = self.remaining();

        while written < len {
            match io.try_write(&self[written..]) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => written += n,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.advance(written);
                    return Ok(true);
                }
                Err(e) => return Err(e.into()),
            }
        }

        self.clear();

        Ok(false)
    }
}

// an internal buffer to collect writes before flushes
pub(super) struct ListWriteBuf<B> {
    /// Re-usable buffer that holds response head.
    /// After head writing finished it's split and pushed to list.
    buf: BytesMut,
    /// Deque of user buffers if strategy is Queue
    list: BufList<B>,
}

impl<B: Buf> Default for ListWriteBuf<B> {
    fn default() -> Self {
        Self {
            buf: BytesMut::new(),
            list: BufList::new(),
        }
    }
}

impl<B: Buf> ListWriteBuf<B> {
    pub(super) fn buffer<BB: Buf + Into<B>>(&mut self, buf: BB) {
        debug_assert!(buf.has_remaining());
        self.list.push(buf.into());
    }
}

impl<B: Buf> fmt::Debug for ListWriteBuf<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteBuf")
            .field("remaining", &self.list.remaining())
            .finish()
    }
}

pub(super) enum EncodedBuf<B> {
    Buf(B),
    Static(&'static [u8]),
}

impl<B: Buf> Buf for EncodedBuf<B> {
    #[inline]
    fn remaining(&self) -> usize {
        match *self {
            Self::Buf(ref buf) => buf.remaining(),
            Self::Static(ref buf) => buf.remaining(),
        }
    }

    #[inline]
    fn chunk(&self) -> &[u8] {
        match *self {
            Self::Buf(ref buf) => buf.chunk(),
            Self::Static(ref buf) => buf.chunk(),
        }
    }

    #[inline]
    fn chunks_vectored<'a>(&'a self, dst: &mut [io::IoSlice<'a>]) -> usize {
        match *self {
            Self::Buf(ref buf) => buf.chunks_vectored(dst),
            Self::Static(ref buf) => buf.chunks_vectored(dst),
        }
    }

    #[inline]
    fn advance(&mut self, cnt: usize) {
        match *self {
            Self::Buf(ref mut buf) => buf.advance(cnt),
            Self::Static(ref mut buf) => buf.advance(cnt),
        }
    }
}

impl<const WRITE_BUF_LIMIT: usize> WriteBuf<WRITE_BUF_LIMIT> for ListWriteBuf<EncodedBuf<Bytes>> {
    #[inline(always)]
    fn len(&self) -> usize {
        // When buffering buf must be empty.
        // (Whoever write into it must split it afterwards)
        debug_assert!(!self.buf.has_remaining());
        self.list.remaining()
    }

    #[inline(always)]
    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        let buf = &mut self.buf;
        let res = func(buf)?;
        let bytes = buf.split().freeze();
        self.buffer(EncodedBuf::Buf(bytes));
        Ok(res)
    }

    #[inline(always)]
    fn write_static(&mut self, bytes: &'static [u8]) {
        self.buffer(EncodedBuf::Static(bytes));
    }

    #[inline(always)]
    fn write_buf(&mut self, bytes: Bytes) {
        self.buffer(EncodedBuf::Buf(bytes));
    }

    #[inline(always)]
    fn write_eof(&mut self, bytes: Bytes) {
        self.buffer(EncodedBuf::Buf(Bytes::from(format!("{:X}\r\n", bytes.len()))));
        self.buffer(EncodedBuf::Buf(bytes));
        self.buffer(EncodedBuf::Static(b"\r\n"));
    }

    #[inline(always)]
    fn try_write_io<Io: AsyncReadWrite>(&mut self, io: &mut Io) -> Result<bool, Error> {
        let queue = &mut self.list;
        while queue.remaining() > 0 {
            let mut iovs = [io::IoSlice::new(&[]); 64];
            let len = queue.chunks_vectored(&mut iovs);
            match io.try_write_vectored(&iovs[..len]) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => queue.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(true);
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(false)
    }
}
