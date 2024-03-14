//! re-export of [tokio::io] types and extended AsyncIo trait on top of it.

// TODO: io module should not re-export tokio types so AsyncIO trait does not depend on runtime
// crate feature.
pub use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};

use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::io;

/// A wrapper trait for an [AsyncRead]/[AsyncWrite] tokio type with additional methods.
pub trait AsyncIo: io::Read + io::Write + Unpin {
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
    fn ready(&self, interest: Interest) -> impl Future<Output = io::Result<Ready>> + Send;

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

/// adapter type for transforming a type impl [AsyncIo] trait to a type impl [AsyncRead] and [AsyncWrite] traits.
/// # Example
/// ```rust
/// use std::{future::poll_fn, pin::Pin};
/// use xitca_io::io::{AsyncIo, AsyncRead, AsyncWrite, PollIoAdapter, ReadBuf};
///
/// async fn adapt(io: impl AsyncIo) {
///     // wrap async io type to adapter.
///     let mut poll_io = PollIoAdapter(io);
///     // use adaptor for polling based io operations.
///     poll_fn(|cx| Pin::new(&mut poll_io).poll_read(cx, &mut ReadBuf::new(&mut [0u8; 1]))).await;
///     poll_fn(|cx| Pin::new(&mut poll_io).poll_write(cx, b"996")).await;    
/// }
/// ```
pub struct PollIoAdapter<T>(pub T)
where
    T: AsyncIo;

impl<T> AsyncRead for PollIoAdapter<T>
where
    T: AsyncIo,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        loop {
            match io::Read::read(&mut this.0, buf.initialize_unfilled()) {
                Ok(n) => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(this.0.poll_ready(Interest::READABLE, cx))?;
                }
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
    }
}

impl<T> AsyncWrite for PollIoAdapter<T>
where
    T: AsyncIo,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        loop {
            match io::Write::write(&mut this.0, buf) {
                Ok(n) => return Poll::Ready(Ok(n)),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(this.0.poll_ready(Interest::WRITABLE, cx))?;
                }
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            match io::Write::flush(&mut this.0) {
                Ok(_) => return Poll::Ready(Ok(())),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(this.0.poll_ready(Interest::WRITABLE, cx))?;
                }
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncIo::poll_shutdown(Pin::new(&mut self.get_mut().0), cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        loop {
            match io::Write::write_vectored(&mut this.0, bufs) {
                Ok(n) => return Poll::Ready(Ok(n)),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(this.0.poll_ready(Interest::WRITABLE, cx))?;
                }
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
    }
    
    fn is_write_vectored(&self) -> bool {
        self.0.is_vectored_write()
    }
}

impl<Io> AsyncIo for PollIoAdapter<Io>
where
    Io: AsyncIo,
{
    #[inline(always)]
    fn ready(&self, interest: Interest) -> impl Future<Output = io::Result<Ready>> + Send {
        self.0.ready(interest)
    }

    #[inline(always)]
    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        self.0.poll_ready(interest, cx)
    }

    fn is_vectored_write(&self) -> bool {
        self.0.is_vectored_write()
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

impl<Io> io::Write for PollIoAdapter<Io>
where
    Io: AsyncIo,
{
    #[inline(always)]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    #[inline(always)]
    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        self.0.write_vectored(bufs)
    }

    #[inline(always)]
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<Io> io::Read for PollIoAdapter<Io>
where
    Io: AsyncIo,
{
    #[inline(always)]
    fn read(&mut self, buf: &mut [u8]) -> ::std::io::Result<usize> {
        self.0.read(buf)
    }
}
