//! Completion-based async IO traits.
//!
//! These traits model IO where buffer ownership is transferred to the operation
//! and returned on completion — the pattern originated from io_uring but not
//! tied to any specific runtime. They can be implemented on top of epoll/kqueue
//! or any other async runtime.

use core::future::Future;

use std::io;

use crate::buf::BoundedBufMut;

/// Async read trait with buffer ownership transfer.
pub trait AsyncBufRead {
    /// Read into a buffer, returning the result and the buffer.
    fn read<B>(&self, buf: B) -> impl Future<Output = (io::Result<usize>, B)>
    where
        B: BoundedBufMut;
}
