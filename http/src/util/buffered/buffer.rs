use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use std::io;

use xitca_io::bytes::{Buf, BytesMut};
use xitca_unsafe_collection::{
    bytes::{BufList, ChunkVectoredUninit},
    uninit::uninit_array,
};

pub use xitca_io::bytes::{BufInterest, BufWrite};

/// a hard code BytesMut that reserving additional 4kb heap memory everytime reallocating needed.
pub type PagedBytesMut = xitca_io::bytes::PagedBytesMut<4096>;

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
        self.0.remaining() < BUF_LIMIT
    }

    fn want_write_io(&self) -> bool {
        unimplemented!()
    }
}

#[derive(Default)]
pub struct WriteBuf<const BUF_LIMIT: usize>(xitca_io::bytes::WriteBuf);

impl<const BUF_LIMIT: usize> WriteBuf<BUF_LIMIT> {
    #[inline]
    pub fn new() -> Self {
        Self(xitca_io::bytes::WriteBuf::new())
    }

    #[cfg(test)]
    pub fn buf(&self) -> &[u8] {
        self.0.buf()
    }
}

impl<const BUF_LIMIT: usize> BufInterest for WriteBuf<BUF_LIMIT> {
    #[inline]
    fn want_write_buf(&self) -> bool {
        self.0.len() < BUF_LIMIT
    }

    #[inline]
    fn want_write_io(&self) -> bool {
        self.0.want_write_io()
    }
}

impl<const BUF_LIMIT: usize> BufWrite for WriteBuf<BUF_LIMIT> {
    #[inline]
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        self.0.write_buf(func)
    }

    #[inline]
    fn do_io<Io: io::Write>(&mut self, io: &mut Io) -> io::Result<()> {
        self.0.do_io(io)
    }
}

// an internal buffer to collect writes before flushes
pub struct ListWriteBuf<B, const BUF_LIMIT: usize> {
    // Re-usable buffer that holds response head.
    // After head writing finished it's split and pushed to list.
    buf: BytesMut,
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
    /// split buf field from Self.
    /// this is often coupled with [ButWrite::write_buf] method to obtain what has been written to
    /// the buf.
    pub fn split_buf(&mut self) -> BytesMut {
        self.buf.split()
    }

    /// add new buf to list.
    ///
    /// # Panics
    /// when push more items to list than the capacity. ListWriteBuf is strictly bounded.
    pub fn buffer<BB: Buf + Into<B>>(&mut self, buf: BB) {
        self.list.push(buf.into());
        // cross reference with <Self as BufWrite>::buf_write method.
        self.want_flush = false;
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
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        // in ListWriteBuf the BytesMut is only used as temporary storage of buffer.
        // only when ListWriteBuf::buffer is called we set self.want_flush to false.
        func(&mut self.buf).map_err(|e| {
            self.buf.clear();
            e
        })
    }

    fn do_io<Io: io::Write>(&mut self, io: &mut Io) -> io::Result<()> {
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
