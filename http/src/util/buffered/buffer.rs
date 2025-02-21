use core::{
    fmt,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};

use std::io;

use tracing::trace;
use xitca_io::bytes::{Buf, BytesMut};
use xitca_unsafe_collection::bytes::{BufList, ChunkVectoredUninit, read_buf};

pub use xitca_io::bytes::{BufInterest, BufRead, BufWrite};

/// a writable buffer with const generic guarded max size limit.
#[derive(Debug)]
pub struct ReadBuf<const LIMIT: usize>(BytesMut);

impl<const LIMIT: usize> ReadBuf<LIMIT> {
    #[inline(always)]
    pub fn new() -> Self {
        Self(BytesMut::new())
    }

    #[inline(always)]
    pub fn into_inner(self) -> BytesMut {
        self.0
    }
}

impl<const LIMIT: usize> From<BytesMut> for ReadBuf<LIMIT> {
    fn from(bytes: BytesMut) -> Self {
        Self(bytes)
    }
}

impl<const LIMIT: usize> Default for ReadBuf<LIMIT> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const LIMIT: usize> Deref for ReadBuf<LIMIT> {
    type Target = BytesMut;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const LIMIT: usize> DerefMut for ReadBuf<LIMIT> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<const LIMIT: usize> BufInterest for ReadBuf<LIMIT> {
    #[inline]
    fn want_write_buf(&self) -> bool {
        self.0.remaining() < LIMIT
    }

    fn want_write_io(&self) -> bool {
        unimplemented!()
    }
}

impl<const LIMIT: usize> BufRead for ReadBuf<LIMIT> {
    fn do_io<Io>(&mut self, io: &mut Io) -> io::Result<()>
    where
        Io: io::Read,
    {
        let len = self.0.len();
        loop {
            match read_buf(io, &mut self.0) {
                Ok(0) => {
                    if self.0.len() == len {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                    break;
                }
                Ok(_) => {
                    if !self.want_write_buf() {
                        trace!(
                            "READ_BUF_LIMIT: {LIMIT} bytes reached. Entering backpressure(no log event for recovery)."
                        );
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    if self.0.len() == len {
                        return Err(e);
                    }
                    break;
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct WriteBuf<const LIMIT: usize>(xitca_io::bytes::WriteBuf);

impl<const LIMIT: usize> WriteBuf<LIMIT> {
    #[inline]
    pub fn new() -> Self {
        Self(xitca_io::bytes::WriteBuf::new())
    }

    #[cfg(test)]
    pub fn buf(&self) -> &[u8] {
        self.0.buf()
    }
}

impl<const LIMIT: usize> BufInterest for WriteBuf<LIMIT> {
    #[inline]
    fn want_write_buf(&self) -> bool {
        self.0.len() < LIMIT
    }

    #[inline]
    fn want_write_io(&self) -> bool {
        self.0.want_write_io()
    }
}

impl<const LIMIT: usize> BufWrite for WriteBuf<LIMIT> {
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
pub struct ListWriteBuf<B, const LIMIT: usize> {
    // Re-usable buffer that holds response head.
    // After head writing finished it's split and pushed to list.
    buf: BytesMut,
    // Deque of user buffers if strategy is Queue
    list: BufList<B, BUF_LIST_CNT>,
    want_flush: bool,
}

impl<B: Buf, const LIMIT: usize> Default for ListWriteBuf<B, LIMIT> {
    fn default() -> Self {
        Self {
            buf: BytesMut::new(),
            list: BufList::new(),
            want_flush: false,
        }
    }
}

impl<B: Buf, const LIMIT: usize> ListWriteBuf<B, LIMIT> {
    /// split buf field from Self.
    /// this is often coupled with [BufWrite::write_buf] method to obtain what has been written to
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

impl<B: Buf, const LIMIT: usize> fmt::Debug for ListWriteBuf<B, LIMIT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListBuf")
            .field("remaining", &self.list.remaining())
            .finish()
    }
}

// buf list is forced to go in backpressure when it reaches this length.
// 32 is chosen for max of 16 pipelined http requests with a single body item.
const BUF_LIST_CNT: usize = 32;

impl<B, const LIMIT: usize> BufInterest for ListWriteBuf<B, LIMIT>
where
    B: Buf + ChunkVectoredUninit,
{
    #[inline]
    fn want_write_buf(&self) -> bool {
        self.list.remaining() < LIMIT && !self.list.is_full()
    }

    #[inline]
    fn want_write_io(&self) -> bool {
        self.list.remaining() != 0 || self.want_flush
    }
}

impl<B, const LIMIT: usize> BufWrite for ListWriteBuf<B, LIMIT>
where
    B: Buf + ChunkVectoredUninit,
{
    fn write_buf<F, T, E>(&mut self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        // in ListWriteBuf the BytesMut is only used as temporary storage of buffer.
        // only when ListWriteBuf::buffer is called we set self.want_flush to false.
        func(&mut self.buf).inspect_err(|_| self.buf.clear())
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

            let mut buf = [const { MaybeUninit::uninit() }; BUF_LIST_CNT];
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
