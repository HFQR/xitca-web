//! Completion-based async IO traits.
//!
//! Re-exported from [`tokio_uring_xitca::io`].

pub use tokio_uring_xitca::buf::{BoundedBuf, BoundedBufMut, Slice};
pub use tokio_uring_xitca::io::{AsyncBufRead, AsyncBufWrite, write_all};
