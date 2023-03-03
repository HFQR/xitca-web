use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use std::io;

use xitca_io::bytes::{Buf, BytesMut};
use xitca_unsafe_collection::{
    bytes::{self, BufList, ChunkVectoredUninit},
    uninit::uninit_array,
};

/// trait generic over different types of buffer strategy.
pub trait BufInterest {
    /// flag if buffer want more data to be filled in.
    fn want_write_buf(&self) -> bool;

    /// flag if buffer want to write data to io.
    fn want_write_io(&self) -> bool;
}

/// trait generic over different types of write buffer strategy.
pub trait BufWrite: BufInterest {
    /// write into buffer write with closure that output a Result type.
    /// the result type is used to hint buffer to stop wanting to flush IO on write_io.
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut Self) -> Result<T, E>;

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
        self.remaining() != 0
    }
}

impl BufWrite for BytesMut {
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut Self) -> Result<T, E>,
    {
        func(self)
    }

    fn write_io<Io: io::Write>(&mut self, _: &mut Io) -> io::Result<()> {
        unimplemented!("<BytesMut as BufWrite>::write_io is not used in server side code.")
    }
}

/// a hard code BytesMut that reserving additional 4kb heap memory everytime reallocating needed.
pub type PagedBytesMut = bytes::PagedBytesMut<4096>;

/// a writable buffer with const generic guarded max size limit.
#[derive(Debug)]
pub struct ReadBuf<const BUF_LIMIT: usize>(PagedBytesMut);

impl<const BUF_LIMIT: usize> ReadBuf<BUF_LIMIT> {
    #[inline]
    pub fn new() -> Self {
        Self(PagedBytesMut::new())
    }
}

impl<const BUF_LIMIT: usize> Default for ReadBuf<BUF_LIMIT> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BUF_LIMIT: usize> Deref for ReadBuf<BUF_LIMIT> {
    type Target = PagedBytesMut;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const BUF_LIMIT: usize> DerefMut for ReadBuf<BUF_LIMIT> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<const BUF_LIMIT: usize> BufInterest for ReadBuf<BUF_LIMIT> {
    #[inline]
    fn want_write_buf(&self) -> bool {
        self.remaining() < BUF_LIMIT
    }

    #[cold]
    #[inline(never)]
    fn want_write_io(&self) -> bool {
        unreachable!("ReadBuf is only meant for reading.")
    }
}

pub struct WriteBuf<const BUF_LIMIT: usize> {
    pub(crate) buf: BytesMut,
    want_flush: bool,
}

impl<const BUF_LIMIT: usize> WriteBuf<BUF_LIMIT> {
    #[inline]
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            want_flush: false,
        }
    }
}

impl<const BUF_LIMIT: usize> From<BytesMut> for WriteBuf<BUF_LIMIT> {
    fn from(buf: BytesMut) -> Self {
        Self { buf, want_flush: false }
    }
}

impl<const BUF_LIMIT: usize> Default for WriteBuf<BUF_LIMIT> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BUF_LIMIT: usize> BufInterest for WriteBuf<BUF_LIMIT> {
    #[inline]
    fn want_write_buf(&self) -> bool {
        self.buf.remaining() < BUF_LIMIT
    }

    #[inline]
    fn want_write_io(&self) -> bool {
        self.buf.remaining() != 0 || self.want_flush
    }
}

impl<const BUF_LIMIT: usize> BufWrite for WriteBuf<BUF_LIMIT> {
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut Self) -> Result<T, E>,
    {
        func(self).map(|t| {
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
                Ok(0) => return write_zero(self.want_write_io()),
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

// an internal buffer to collect writes before flushes
pub struct ListWriteBuf<B, const BUF_LIMIT: usize> {
    // Re-usable buffer that holds response head.
    // After head writing finished it's split and pushed to list.
    pub buf: BytesMut,
    // Deque of user buffers if strategy is Queue
    list: BufList<B, BUF_LIST_CNT>,
    want_flush: bool,
}

impl<B: Buf, const BUF_LIMIT: usize> Default for ListWriteBuf<B, BUF_LIMIT> {
    fn default() -> Self {
        Self {
            buf: BytesMut::new(),
            list: BufList::new(),
            want_flush: false,
        }
    }
}

impl<B: Buf, const BUF_LIMIT: usize> ListWriteBuf<B, BUF_LIMIT> {
    pub fn buffer<BB: Buf + Into<B>>(&mut self, buf: BB) {
        self.list.push(buf.into());
    }
}

impl<B: Buf, const BUF_LIMIT: usize> fmt::Debug for ListWriteBuf<B, BUF_LIMIT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListBuf")
            .field("remaining", &self.list.remaining())
            .finish()
    }
}

// buf list is forced to go in backpressure when it reaches this length.
// 32 is chosen for max of 16 pipelined http requests with a single body item.
const BUF_LIST_CNT: usize = 32;

impl<B, const BUF_LIMIT: usize> BufInterest for ListWriteBuf<B, BUF_LIMIT>
where
    B: Buf + ChunkVectoredUninit,
{
    #[inline]
    fn want_write_buf(&self) -> bool {
        self.list.remaining() < BUF_LIMIT && !self.list.is_full()
    }

    #[inline]
    fn want_write_io(&self) -> bool {
        self.list.remaining() != 0 || self.want_flush
    }
}

impl<B, const BUF_LIMIT: usize> BufWrite for ListWriteBuf<B, BUF_LIMIT>
where
    B: Buf + ChunkVectoredUninit,
{
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut Self) -> Result<T, E>,
    {
        func(self).map(|t| {
            self.want_flush = false;
            t
        })
    }

    fn write_io<Io: io::Write>(&mut self, io: &mut Io) -> io::Result<()> {
        let queue = &mut self.list;

        loop {
            if self.want_flush {
                match io::Write::flush(io) {
                    Ok(_) => self.want_flush = false,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                break;
            }

            let mut buf = uninit_array::<_, BUF_LIST_CNT>();
            let slice = queue.chunks_vectored_uninit_into_init(&mut buf);
            match io.write_vectored(slice) {
                Ok(0) => return write_zero(self.want_write_io()),
                Ok(n) => {
                    queue.advance(n);
                    if queue.is_empty() {
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

#[cold]
#[inline(never)]
fn write_zero(want_write: bool) -> io::Result<()> {
    assert!(
        want_write,
        "BufWrite::write must be called after BufInterest::want_write return true."
    );
    Err(io::ErrorKind::WriteZero.into())
}
