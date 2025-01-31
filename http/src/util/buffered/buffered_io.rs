use core::{
    future::{Future, poll_fn},
    pin::Pin,
};

use std::io;

use xitca_io::io::{AsyncIo, Interest};

use super::buffer::{BufRead, BufWrite, ReadBuf};

/// Io type with internal buffering.
pub struct BufferedIo<'a, St, W, const READ_BUF_LIMIT: usize> {
    /// mut reference of Io type that impl [AsyncIo] trait.
    pub io: &'a mut St,
    /// read buffer with const generic usize as capacity limit.
    pub read_buf: ReadBuf<READ_BUF_LIMIT>,
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
            read_buf: ReadBuf::new(),
            write_buf,
        }
    }

    /// read until io blocked or read buffer is full and advance the length of it(read buffer).
    #[inline]
    pub fn try_read(&mut self) -> io::Result<()> {
        BufRead::do_io(&mut self.read_buf, self.io)
    }

    /// write until write buffer is emptied or io blocked.
    #[inline]
    pub fn try_write(&mut self) -> io::Result<()> {
        BufWrite::do_io(&mut self.write_buf, self.io)
    }

    /// check for io read readiness in async and do [Self::try_read].
    pub async fn read(&mut self) -> io::Result<()> {
        self.io.ready(Interest::READABLE).await?;
        self.try_read()
    }

    /// drain write buffer and flush the io.
    pub async fn drain_write(&mut self) -> io::Result<()> {
        while self.write_buf.want_write_io() {
            self.io.ready(Interest::WRITABLE).await?;
            self.try_write()?;
        }
        Ok(())
    }

    /// shutdown Io gracefully.
    pub fn shutdown(&mut self) -> impl Future<Output = io::Result<()>> + '_ {
        poll_fn(|cx| Pin::new(&mut *self.io).poll_shutdown(cx))
    }
}
