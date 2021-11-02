use std::{
    fmt,
    io::{self, Write},
    ops::{Deref, DerefMut},
};

use xitca_io::io::AsyncIo;

use crate::{
    bytes::{buf::Chain, Buf, BufMut, Bytes, BytesMut},
    h1::error::Error,
    util::{buf_list::BufList, writer::Writer},
};

// buf list is forced to go in backpressure when it reaches this length.
// 32 is chosen for max of 16 pipelined http requests with a single body item.
const BUF_LIST_CNT: usize = 32;

/// Trait to generic over different types of write buffer strategy.
pub trait WriteBuf {
    fn backpressure(&self) -> bool;

    fn is_empty(&self) -> bool;

    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>;

    fn write_static(&mut self, bytes: &'static [u8]);

    fn write_buf(&mut self, bytes: Bytes);

    fn write_chunk(&mut self, bytes: Bytes);

    fn try_write_io<Io: AsyncIo, E>(&mut self, io: &mut Io) -> Result<(), Error<E>>;
}

pub struct FlatBuf<const BUF_LIMIT: usize>(BytesMut);

impl<const BUF_LIMIT: usize> FlatBuf<BUF_LIMIT> {
    #[inline]
    pub fn new() -> Self {
        Self(BytesMut::new())
    }
}

impl<const BUF_LIMIT: usize> From<BytesMut> for FlatBuf<BUF_LIMIT> {
    fn from(bytes_mut: BytesMut) -> Self {
        Self(bytes_mut)
    }
}

impl<const BUF_LIMIT: usize> Default for FlatBuf<BUF_LIMIT> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BUF_LIMIT: usize> Deref for FlatBuf<BUF_LIMIT> {
    type Target = BytesMut;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const BUF_LIMIT: usize> DerefMut for FlatBuf<BUF_LIMIT> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<const BUF_LIMIT: usize> WriteBuf for FlatBuf<BUF_LIMIT> {
    #[inline]
    fn backpressure(&self) -> bool {
        self.remaining() >= BUF_LIMIT
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    #[inline]
    fn write_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        func(&mut *self)
    }

    #[inline]
    fn write_static(&mut self, bytes: &'static [u8]) {
        self.put_slice(bytes);
    }

    #[inline]
    fn write_buf(&mut self, bytes: Bytes) {
        self.put_slice(bytes.as_ref());
    }

    fn write_chunk(&mut self, bytes: Bytes) {
        write!(Writer::new(&mut **self), "{:X}\r\n", bytes.len()).unwrap();

        self.reserve(bytes.len() + 2);
        self.put_slice(bytes.as_ref());
        self.put_slice(b"\r\n");
    }

    fn try_write_io<Io: AsyncIo, E>(&mut self, io: &mut Io) -> Result<(), Error<E>> {
        let mut written = 0;
        let len = self.remaining();

        while written < len {
            match io.try_write(&self[written..]) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => written += n,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.advance(written);
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            }
        }

        self.clear();

        Ok(())
    }
}

// an internal buffer to collect writes before flushes
pub(super) struct ListBuf<B, const BUF_LIMIT: usize> {
    /// Re-usable buffer that holds response head.
    /// After head writing finished it's split and pushed to list.
    buf: BytesMut,
    /// Deque of user buffers if strategy is Queue
    list: BufList<B>,
}

impl<B: Buf, const BUF_LIMIT: usize> Default for ListBuf<B, BUF_LIMIT> {
    fn default() -> Self {
        Self {
            buf: BytesMut::new(),
            list: BufList::with_capacity(BUF_LIMIT),
        }
    }
}

impl<B: Buf, const BUF_LIMIT: usize> ListBuf<B, BUF_LIMIT> {
    pub(super) fn buffer<BB: Buf + Into<B>>(&mut self, buf: BB) {
        debug_assert!(buf.has_remaining());
        self.list.push(buf.into());
    }
}

impl<B: Buf, const BUF_LIMIT: usize> fmt::Debug for ListBuf<B, BUF_LIMIT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteBuf")
            .field("remaining", &self.list.remaining())
            .finish()
    }
}

pub(super) enum EncodedBuf<B, BB> {
    Buf(B),
    Chunk(BB),
    Static(&'static [u8]),
}

impl<B: Buf, BB: Buf> Buf for EncodedBuf<B, BB> {
    #[inline]
    fn remaining(&self) -> usize {
        match *self {
            Self::Buf(ref buf) => buf.remaining(),
            Self::Chunk(ref buf) => buf.remaining(),
            Self::Static(ref buf) => buf.remaining(),
        }
    }

    #[inline]
    fn chunk(&self) -> &[u8] {
        match *self {
            Self::Buf(ref buf) => buf.chunk(),
            Self::Chunk(ref buf) => buf.chunk(),
            Self::Static(ref buf) => buf.chunk(),
        }
    }

    #[inline]
    fn chunks_vectored<'a>(&'a self, dst: &mut [io::IoSlice<'a>]) -> usize {
        match *self {
            Self::Buf(ref buf) => buf.chunks_vectored(dst),
            Self::Chunk(ref buf) => buf.chunks_vectored(dst),
            Self::Static(ref buf) => buf.chunks_vectored(dst),
        }
    }

    #[inline]
    fn advance(&mut self, cnt: usize) {
        match *self {
            Self::Buf(ref mut buf) => buf.advance(cnt),
            Self::Chunk(ref mut buf) => buf.advance(cnt),
            Self::Static(ref mut buf) => buf.advance(cnt),
        }
    }
}

// as special type for eof chunk when using transfer-encoding: chunked
type Eof = Chain<Chain<Bytes, Bytes>, &'static [u8]>;

impl<const BUF_LIMIT: usize> WriteBuf for ListBuf<EncodedBuf<Bytes, Eof>, BUF_LIMIT> {
    #[inline]
    fn backpressure(&self) -> bool {
        self.list.remaining() >= BUF_LIMIT || self.list.cnt() == BUF_LIST_CNT
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.list.remaining() == 0
    }

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

    #[inline]
    fn write_static(&mut self, bytes: &'static [u8]) {
        self.buffer(EncodedBuf::Static(bytes));
    }

    fn write_buf(&mut self, bytes: Bytes) {
        self.buffer(EncodedBuf::Buf(bytes));
    }

    #[inline]
    fn write_chunk(&mut self, bytes: Bytes) {
        let eof = Bytes::from(format!("{:X}\r\n", bytes.len()))
            .chain(bytes)
            .chain(b"\r\n" as &'static [u8]);

        self.buffer(EncodedBuf::Chunk(eof));
    }

    fn try_write_io<Io: AsyncIo, E>(&mut self, io: &mut Io) -> Result<(), Error<E>> {
        let queue = &mut self.list;
        while queue.remaining() > 0 {
            let mut iovs = [io::IoSlice::new(&[]); BUF_LIST_CNT];
            let len = queue.chunks_vectored(&mut iovs);
            match io.try_write_vectored(&iovs[..len]) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => queue.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }
}
