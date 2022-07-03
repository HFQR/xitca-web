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

/// Trait for different types of read/write buffer strategy's interests.
pub trait BufInterest {
    // TODO: rename to want_buf
    fn backpressure(&self) -> bool;
    fn want_write(&self) -> bool;
}

/// Trait to generic over different types of write buffer strategy.
pub trait BufWrite: BufInterest {
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

    /// flush buf and write to given async io.
    fn flush<Io: AsyncIo>(&mut self, io: &mut Io) -> io::Result<()>;
}

pub struct FlatBuf<const BUF_LIMIT: usize> {
    inner: BytesMut,
    flush: Flush,
}

impl<const BUF_LIMIT: usize> FlatBuf<BUF_LIMIT> {
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: BytesMut::new(),
            flush: Default::default(),
        }
    }
}

impl<const BUF_LIMIT: usize> From<BytesMut> for FlatBuf<BUF_LIMIT> {
    fn from(bytes_mut: BytesMut) -> Self {
        Self {
            inner: bytes_mut,
            flush: Default::default(),
        }
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
        &self.inner
    }
}

impl<const BUF_LIMIT: usize> DerefMut for FlatBuf<BUF_LIMIT> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<const BUF_LIMIT: usize> BufInterest for FlatBuf<BUF_LIMIT> {
    #[inline]
    fn backpressure(&self) -> bool {
        self.remaining() >= BUF_LIMIT
    }

    #[inline]
    fn want_write(&self) -> bool {
        self.remaining() != 0 || self.flush.want_flush()
    }
}

impl<const BUF_LIMIT: usize> BufWrite for FlatBuf<BUF_LIMIT> {
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
    fn write_bytes(&mut self, bytes: Bytes) {
        self.put_slice(bytes.as_ref());
    }

    fn write_chunked(&mut self, bytes: Bytes) {
        write!(BufMutWriter(&mut **self), "{:X}\r\n", bytes.len()).unwrap();

        self.reserve(bytes.len() + 2);
        self.put_slice(bytes.as_ref());
        self.put_slice(b"\r\n");
    }

    fn flush<Io: AsyncIo>(&mut self, io: &mut Io) -> io::Result<()> {
        if !self.flush.want_flush() {
            let mut written = 0;
            let len = self.remaining();

            while written < len {
                match io.write(&self[written..]) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => written += n,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                }
            }

            self.advance(written);
        }

        self.flush.flush(io)
    }
}

// an internal buffer to collect writes before flushes
pub(super) struct ListBuf<B, const BUF_LIMIT: usize> {
    // Re-usable buffer that holds response head.
    // After head writing finished it's split and pushed to list.
    buf: BytesMut,
    // Deque of user buffers if strategy is Queue
    list: BufList<B, BUF_LIST_CNT>,
    flush: Flush,
}

impl<B: Buf, const BUF_LIMIT: usize> Default for ListBuf<B, BUF_LIMIT> {
    fn default() -> Self {
        Self {
            buf: BytesMut::new(),
            list: BufList::new(),
            flush: Default::default(),
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

impl<const BUF_LIMIT: usize> BufInterest for ListBuf<EncodedBuf<Bytes, Eof>, BUF_LIMIT> {
    #[inline]
    fn backpressure(&self) -> bool {
        self.list.remaining() >= BUF_LIMIT || self.list.is_full()
    }

    #[inline]
    fn want_write(&self) -> bool {
        self.list.remaining() != 0 || self.flush.want_flush()
    }
}

impl<const BUF_LIMIT: usize> BufWrite for ListBuf<EncodedBuf<Bytes, Eof>, BUF_LIMIT> {
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

    fn flush<Io: AsyncIo>(&mut self, io: &mut Io) -> io::Result<()> {
        if !self.flush.want_flush() {
            let queue = &mut self.list;
            while !queue.is_empty() {
                let mut buf = uninit::uninit_array::<_, BUF_LIST_CNT>();
                let slice = queue.chunks_vectored_uninit_into_init(&mut buf);

                match io.write_vectored(slice) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => queue.advance(n),
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                }
            }
        }

        self.flush.flush(io)
    }
}

// a track type for given io's flush state.
struct Flush(bool);

impl Default for Flush {
    fn default() -> Self {
        Self(false)
    }
}

impl Flush {
    // if flush want to write to io
    fn want_flush(&self) -> bool {
        self.0
    }

    // flush io and set self to want_flush when flush is blocked.
    fn flush<Io: AsyncIo>(&mut self, io: &mut Io) -> io::Result<()> {
        match io.flush() {
            Ok(_) => self.0 = false,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => self.0 = true,
            Err(e) => return Err(e),
        }

        Ok(())
    }
}
