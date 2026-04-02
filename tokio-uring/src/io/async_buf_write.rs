//! Completion-based async IO traits.
//!
//! These traits model IO where buffer ownership is transferred to the operation
//! and returned on completion — the pattern originated from io_uring but not
//! tied to any specific runtime. They can be implemented on top of epoll/kqueue
//! or any other async runtime.

use core::future::Future;

use std::{io, net::Shutdown};

use crate::buf::{BoundedBuf, IoBuf};

/// Async write trait with buffer ownership transfer.
pub trait AsyncBufWrite {
    /// Write from a buffer, returning the result and the buffer.
    fn write<B>(&self, buf: B) -> impl Future<Output = (io::Result<usize>, B)>
    where
        B: BoundedBuf;

    /// Write all bytes from a buffer to IO.
    fn write_all<B>(&self, buf: B) -> impl Future<Output = (io::Result<()>, B)>
    where
        B: IoBuf,
        Self: Sized,
    {
        write_all(self, buf)
    }

    /// Shutdown the connection in the given direction.
    ///
    /// Takes ownership of `self` because shutdown is a terminal operation.
    /// No further reads or writes should occur after shutdown.
    fn shutdown(self, direction: Shutdown) -> impl Future<Output = io::Result<()>>;
}

/// Write all bytes from a buffer to IO.
pub async fn write_all<Io, B>(io: &Io, buf: B) -> (io::Result<()>, B)
where
    Io: AsyncBufWrite,
    B: IoBuf,
{
    let mut buf = buf.slice_full();
    while buf.bytes_init() != 0 {
        match io.write(buf).await {
            (Ok(0), slice) => {
                return (Err(io::ErrorKind::WriteZero.into()), slice.into_inner());
            }
            (Ok(n), slice) => buf = slice.slice(n..),
            (Err(e), slice) => {
                return (Err(e), slice.into_inner());
            }
        }
    }
    (Ok(()), buf.into_inner())
}
