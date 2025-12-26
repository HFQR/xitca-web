use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

pub use openssl::*;

use openssl::ssl::{ErrorCode, ShutdownResult, Ssl, SslRef, SslStream};
use xitca_io::io::{AsyncIo, Interest, Ready};

/// A stream managed by `openssl` crate for tls read/write.
pub struct TlsStream<Io> {
    io: SslStream<Io>,
}

impl<Io> TlsStream<Io>
where
    Io: AsyncIo,
{
    /// acquire a reference to the session type.
    pub fn session(&self) -> &SslRef {
        self.io.ssl()
    }

    pub async fn accept(ssl: Ssl, io: Io) -> Result<Self, Error> {
        Self::connect_or_accept(ssl, io, |io| io.accept()).await
    }

    pub async fn connect(ssl: Ssl, io: Io) -> Result<Self, Error> {
        Self::connect_or_accept(ssl, io, |io| io.connect()).await
    }

    async fn connect_or_accept<F>(ssl: Ssl, io: Io, mut func: F) -> Result<Self, Error>
    where
        F: FnMut(&mut SslStream<Io>) -> Result<(), openssl::ssl::Error>,
    {
        let mut io = SslStream::new(ssl, io)?;
        let mut interest = Interest::READABLE | Interest::WRITABLE;
        loop {
            io.get_mut().ready(interest).await.map_err(Error::Io)?;
            match func(&mut io) {
                Ok(_) => return Ok(TlsStream { io }),
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    interest = Interest::READABLE;
                }
                Err(ref e) if e.code() == ErrorCode::WANT_WRITE => {
                    interest = Interest::WRITABLE;
                }
                Err(e) => return Err(Error::Tls(e)),
            }
        }
    }
}

impl<Io: AsyncIo> AsyncIo for TlsStream<Io> {
    #[inline]
    fn ready(&mut self, interest: Interest) -> impl Future<Output = io::Result<Ready>> + Send {
        self.io.get_mut().ready(interest)
    }

    #[inline]
    fn poll_ready(&mut self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        self.io.get_mut().poll_ready(interest, cx)
    }

    fn is_vectored_write(&self) -> bool {
        self.io.get_ref().is_vectored_write()
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        // copied from tokio-openssl crate.
        match this.io.shutdown() {
            Ok(ShutdownResult::Sent) | Ok(ShutdownResult::Received) => {}
            Err(ref e) if e.code() == ErrorCode::ZERO_RETURN => {}
            Err(ref e) if e.code() == ErrorCode::WANT_READ || e.code() == ErrorCode::WANT_WRITE => {
                return Poll::Pending;
            }
            Err(e) => {
                return Poll::Ready(Err(e.into_io_error().unwrap_or_else(io::Error::other)));
            }
        }

        AsyncIo::poll_shutdown(Pin::new(this.io.get_mut()), cx)
    }
}

impl<Io: AsyncIo> io::Read for TlsStream<Io> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.io, buf)
    }
}

impl<Io: AsyncIo> io::Write for TlsStream<Io> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(&mut self.io, buf)
    }

    #[inline]
    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        io::Write::write_vectored(&mut self.io, bufs)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.io)
    }
}

/// Collection of 'openssl' error types.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Tls(openssl::ssl::Error),
}

impl From<openssl::error::ErrorStack> for Error {
    fn from(e: openssl::error::ErrorStack) -> Self {
        Self::Tls(e.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => fmt::Display::fmt(e, f),
            Self::Tls(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for Error {}
