//! Completion-based async IO traits.
//!
//! These traits model IO where buffer ownership is transferred to the operation
//! and returned on completion — the pattern originated from io_uring but not
//! tied to any specific runtime. They can be implemented on top of epoll/kqueue
//! or any other async runtime.

use core::future::Future;

use std::{io, net::Shutdown};

use tokio_uring_xitca::buf::IoBuf;

pub use tokio_uring_xitca::buf::{BoundedBuf, BoundedBufMut, Slice};

/// Async read trait with buffer ownership transfer.
pub trait AsyncBufRead {
    /// Read into a buffer, returning the result and the buffer.
    fn read<B>(&self, buf: B) -> impl Future<Output = (io::Result<usize>, B)>
    where
        B: BoundedBufMut;
}

/// Async write trait with buffer ownership transfer.
pub trait AsyncBufWrite {
    /// Write from a buffer, returning the result and the buffer.
    fn write<B>(&self, buf: B) -> impl Future<Output = (io::Result<usize>, B)>
    where
        B: BoundedBuf;

    /// Shutdown the connection in the given direction.
    fn shutdown(&self, direction: Shutdown) -> impl Future<Output = io::Result<()>>;
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
