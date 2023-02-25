//! re-export of [tokio::io] types and extended AsyncIo trait on top of it.

// TODO: io module should not re-export tokio types so AsyncIO trait does not depend on runtime
// crate feature.
pub use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

/// A wrapper trait for an [AsyncRead]/[AsyncWrite] tokio type with additional methods.
pub trait AsyncIo: io::Read + io::Write + Unpin {
    type Future<'f>: Future<Output = io::Result<Ready>>
    where
        Self: 'f;

    /// asynchronously wait for the IO type and return it's state as [Ready].
    ///
    /// # Errors:
    ///
    /// The only error cause of ready should be from runtime shutdown. Indicates no further
    /// operations can be done.
    ///
    /// Actual IO error should be exposed from [std::io::Read]/[std::io::Write] methods.
    ///
    /// This constraint is from `tokio`'s behavior which is what xitca built upon and rely on
    /// in downstream crates like `xitca-http` etc.
    fn ready(&self, interest: Interest) -> Self::Future<'_>;

    /// a poll version of ready method.
    ///
    /// # Why:
    /// This is a temporary method for backward compat of [AsyncRead] and [AsyncWrite] traits.
    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>>;

    /// hint if IO can be vectored write.
    ///
    /// # Why:
    /// std `can_vector` feature is not stabled yet and xitca make use of vectored io write.
    fn is_vectored_write(&self) -> bool;

    /// poll shutdown the write part of Self.
    ///
    /// # Why:
    /// tokio's network Stream types do not expose other api for shutdown besides [AsyncWrite::poll_shutdown].
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
}
