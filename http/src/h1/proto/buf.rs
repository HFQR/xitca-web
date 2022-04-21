use std::{
    fmt,
    io::{self, Write},
    ops::{Deref, DerefMut},
};

use xitca_io::io::AsyncIo;
use xitca_unsafe_collection::{
    bytes::{BufList, EitherBuf},
    uninit,
};

use crate::bytes::{buf::Chain, Buf, BufMut, BufMutWriter, Bytes, BytesMut};

/// Trait for bound check different types of read/write buffer strategy.
pub trait BufBound {
    fn backpressure(&self) -> bool;
    fn is_empty(&self) -> bool;
}

/// Trait to generic over different types of write buffer strategy.
pub trait BufWrite: BufBound {
    fn buf_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>;

    fn buf_static(&mut self, bytes: &'static [u8]);

    fn buf_bytes(&mut self, bytes: Bytes);

    fn buf_chunked(&mut self, bytes: Bytes);

    fn try_write<Io: AsyncIo>(&mut self, io: &mut Io) -> io::Result<()>;
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

impl<const BUF_LIMIT: usize> BufBound for FlatBuf<BUF_LIMIT> {
    #[inline]
    fn backpressure(&self) -> bool {
        self.remaining() >= BUF_LIMIT
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }
}

impl<const BUF_LIMIT: usize> BufWrite for FlatBuf<BUF_LIMIT> {
    #[inline]
    fn buf_head<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        func(&mut *self)
    }

    #[inline]
    fn buf_static(&mut self, bytes: &'static [u8]) {
        self.put_slice(bytes);
    }

    #[inline]
    fn buf_bytes(&mut self, bytes: Bytes) {
        self.put_slice(bytes.as_ref());
    }

    fn buf_chunked(&mut self, bytes: Bytes) {
        write!(BufMutWriter(&mut **self), "{:X}\r\n", bytes.len()).unwrap();

        self.reserve(bytes.len() + 2);
        self.put_slice(bytes.as_ref());
        self.put_slice(b"\r\n");
    }

    fn try_write<Io: AsyncIo>(&mut self, io: &mut Io) -> io::Result<()> {
        let mut written = 0;
        let len = self.remaining();

        while written < len {
            match io.try_write(&self[written..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => written += n,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.advance(written);
                    return Ok(());
                }
                Err(e) => return Err(e),
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
    list: BufList<B, BUF_LIST_CNT>,
}

impl<B: Buf, const BUF_LIMIT: usize> Default for ListBuf<B, BUF_LIMIT> {
    fn default() -> Self {
        Self {
            buf: BytesMut::new(),
            list: BufList::new(),
        }
    }
}

impl<B: Buf, const BUF_LIMIT: usize> ListBuf<B, BUF_LIMIT> {
    pub(super) fn buffer<BB: Buf + Into<B>>(&mut self, buf: BB) {
        self.list.push(buf.into());
    }
}

impl<B: Buf, const BUF_LIMIT: usize> fmt::Debug for ListBuf<B, BUF_LIMIT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListBuf")
            .field("remaining", &self.list.remaining())
            .finish()
    }
}

// as special type for eof chunk when using transfer-encoding: chunked
type Eof = Chain<Chain<Bytes, Bytes>, &'static [u8]>;

type EncodedBuf<B, B2> = EitherBuf<B, EitherBuf<B2, &'static [u8]>>;

// buf list is forced to go in backpressure when it reaches this length.
// 32 is chosen for max of 16 pipelined http requests with a single body item.
const BUF_LIST_CNT: usize = 32;

impl<const BUF_LIMIT: usize> BufBound for ListBuf<EncodedBuf<Bytes, Eof>, BUF_LIMIT> {
    #[inline]
    fn backpressure(&self) -> bool {
        self.list.remaining() >= BUF_LIMIT || self.list.is_full()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.list.remaining() == 0
    }
}

impl<const BUF_LIMIT: usize> BufWrite for ListBuf<EncodedBuf<Bytes, Eof>, BUF_LIMIT> {
    fn buf_head<F, T, E>(&mut self, func: F) -> Result<T, E>
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
    fn buf_static(&mut self, bytes: &'static [u8]) {
        self.buffer(EitherBuf::Right(EitherBuf::Right(bytes)));
    }

    #[inline]
    fn buf_bytes(&mut self, bytes: Bytes) {
        self.buffer(EitherBuf::Left(bytes));
    }

    #[inline]
    fn buf_chunked(&mut self, bytes: Bytes) {
        let eof = Bytes::from(format!("{:X}\r\n", bytes.len()))
            .chain(bytes)
            .chain(b"\r\n" as &'static [u8]);

        self.buffer(EitherBuf::Right(EitherBuf::Left(eof)));
    }

    fn try_write<Io: AsyncIo>(&mut self, io: &mut Io) -> io::Result<()> {
        let queue = &mut self.list;
        while !queue.is_empty() {
            let mut buf = uninit::uninit_array::<_, BUF_LIST_CNT>();
            let slice = queue.chunks_vectored_uninit_into_init(&mut buf);

            match io.try_write_vectored(slice) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => queue.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }
}
