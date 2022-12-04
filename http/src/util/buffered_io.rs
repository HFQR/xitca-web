use std::{
    fmt,
    future::poll_fn,
    io::{self, Write},
    ops::{Deref, DerefMut},
    pin::Pin,
};

use tracing::trace;
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::{
    bytes::{read_buf, BufList, ChunkVectoredUninit},
    uninit::uninit_array,
};

/// Io type with internal buffering.
pub struct BufferedIo<'a, St, W, const READ_BUF_LIMIT: usize> {
    /// mut reference of Io type that impl [AsyncIo] trait.
    pub io: &'a mut St,
    /// read buffer with const generic usize as capacity limit.
    pub read_buf: FlatBuf<READ_BUF_LIMIT>,
    /// generic type impl [BufWrite] trait as write buffer.
    pub write_buf: W,
}

impl<'a, St, W, const READ_BUF_LIMIT: usize> BufferedIo<'a, St, W, READ_BUF_LIMIT>
where
    St: AsyncIo,
    W: BufWrite,
{
    /// construct a new buffered io with given Io and buf writer.
    pub fn new(io: &'a mut St, write_buf: W) -> Self {
        Self {
            io,
            read_buf: FlatBuf::new(),
            write_buf,
        }
    }

    /// read until io blocked or read buffer is full and advance the length of it(read buffer).
    pub fn try_read(&mut self) -> io::Result<()> {
        loop {
            match read_buf(self.io, &mut *self.read_buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(_) => {
                    if !self.read_buf.want_buf() {
                        trace!(target: "dispatchter", "Read buffer limit reached(Current length: {} bytes). Entering backpressure(No log event for recovery).", self.read_buf.len());
                        return Ok(());
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    /// write until write buffer is emptied or io blocked.
    pub fn try_write(&mut self) -> io::Result<()> {
        self.write_buf.write(self.io)
    }

    /// check for io read readiness in async and do [Self::try_write].
    pub async fn read(&mut self) -> io::Result<()> {
        self.io.ready(Interest::READABLE).await?;
        self.try_read()
    }

    /// drain write buffer and flush the io.
    pub async fn drain_write(&mut self) -> io::Result<()> {
        while self.write_buf.want_write() {
            self.io.ready(Interest::WRITABLE).await?;
            self.try_write()?;
        }

        loop {
            match Write::flush(&mut self.io) {
                Ok(()) => return Ok(()),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            self.io.ready(Interest::WRITABLE).await?;
        }
    }

    /// shutdown Io gracefully.
    #[cold]
    #[inline(never)]
    pub async fn shutdown(&mut self) -> io::Result<()> {
        poll_fn(|cx| Pin::new(&mut *self.io).poll_shutdown(cx)).await
    }
}

/// trait generic over different types of buffer strategy.
pub trait BufInterest {
    /// flag if buffer want more data to be filled in.
    fn want_buf(&self) -> bool;

    /// flag if buffer want to write data to io.
    fn want_write(&self) -> bool;
}

/// trait generic over different types of write buffer strategy.
pub trait BufWrite: BufInterest {
    /// write buffer to given io.
    fn write<Io: Write>(&mut self, io: &mut Io) -> io::Result<()>;
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

impl<const BUF_LIMIT: usize> BufInterest for FlatBuf<BUF_LIMIT> {
    #[inline]
    fn want_buf(&self) -> bool {
        self.remaining() < BUF_LIMIT
    }

    #[inline(always)]
    fn want_write(&self) -> bool {
        self.0.want_write()
    }
}

impl<const BUF_LIMIT: usize> BufWrite for FlatBuf<BUF_LIMIT> {
    #[inline(always)]
    fn write<Io: Write>(&mut self, io: &mut Io) -> io::Result<()> {
        self.0.write(io)
    }
}

impl BufInterest for BytesMut {
    #[inline]
    fn want_buf(&self) -> bool {
        true
    }

    #[inline]
    fn want_write(&self) -> bool {
        self.remaining() != 0
    }
}

impl BufWrite for BytesMut {
    fn write<Io: Write>(&mut self, io: &mut Io) -> io::Result<()> {
        loop {
            match io.write(self) {
                Ok(0) => return write_zero(self.want_write()),
                Ok(n) => {
                    self.advance(n);
                    if self.is_empty() {
                        break;
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
pub struct ListBuf<B, const BUF_LIMIT: usize> {
    // Re-usable buffer that holds response head.
    // After head writing finished it's split and pushed to list.
    pub buf: BytesMut,
    // Deque of user buffers if strategy is Queue
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
    pub fn buffer<BB: Buf + Into<B>>(&mut self, buf: BB) {
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

// buf list is forced to go in backpressure when it reaches this length.
// 32 is chosen for max of 16 pipelined http requests with a single body item.
const BUF_LIST_CNT: usize = 32;

impl<B, const BUF_LIMIT: usize> BufInterest for ListBuf<B, BUF_LIMIT>
where
    B: Buf + ChunkVectoredUninit,
{
    #[inline]
    fn want_buf(&self) -> bool {
        self.list.remaining() < BUF_LIMIT && !self.list.is_full()
    }

    #[inline]
    fn want_write(&self) -> bool {
        self.list.remaining() != 0
    }
}

impl<B, const BUF_LIMIT: usize> BufWrite for ListBuf<B, BUF_LIMIT>
where
    B: Buf + ChunkVectoredUninit,
{
    fn write<Io: Write>(&mut self, io: &mut Io) -> io::Result<()> {
        let queue = &mut self.list;
        loop {
            let mut buf = uninit_array::<_, BUF_LIST_CNT>();
            let slice = queue.chunks_vectored_uninit_into_init(&mut buf);
            match io.write_vectored(slice) {
                Ok(0) => return write_zero(self.want_write()),
                Ok(n) => {
                    queue.advance(n);
                    if queue.is_empty() {
                        break;
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
