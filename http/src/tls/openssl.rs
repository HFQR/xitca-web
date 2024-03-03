pub(crate) use openssl::ssl::SslAcceptor as TlsAcceptor;

use core::{
    convert::Infallible,
    fmt::{self, Debug, Formatter},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

use openssl::{
    error::ErrorStack,
    ssl::{Error, ErrorCode, ShutdownResult, Ssl, SslStream},
};
use xitca_io::io::{AsyncIo, AsyncReadWrite, Interest, Ready};
use xitca_service::Service;

use crate::{http::Version, version::AsVersion};

use super::error::TlsError;

/// A wrapper type for [SslStream].
///
/// This is to impl new trait for it.
pub struct TlsStream<Io> {
    io: SslStream<Io>,
}

impl<Io> AsVersion for AsyncReadWrite<TlsStream<Io>>
where
    Io: AsyncIo,
{
    fn as_version(&self) -> Version {
        self.0
            .io
            .ssl()
            .selected_alpn_protocol()
            .map(Self::from_alpn)
            .unwrap_or(Version::HTTP_11)
    }
}

#[derive(Clone)]
pub struct TlsAcceptorBuilder {
    acceptor: TlsAcceptor,
}

impl TlsAcceptorBuilder {
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Self { acceptor }
    }
}

impl Service for TlsAcceptorBuilder {
    type Response = TlsAcceptorService;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        let service = TlsAcceptorService {
            acceptor: self.acceptor.clone(),
        };
        Ok(service)
    }
}

/// Openssl Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
pub struct TlsAcceptorService {
    acceptor: TlsAcceptor,
}

impl TlsAcceptorService {
    #[inline(never)]
    async fn accept<Io: AsyncIo>(&self, io: Io) -> Result<AsyncReadWrite<TlsStream<Io>>, OpensslError> {
        let ctx = self.acceptor.context();
        let ssl = Ssl::new(ctx)?;
        let mut io = SslStream::new(ssl, io)?;
        let mut interest = Interest::READABLE;
        loop {
            io.get_mut().ready(interest).await?;
            match io.accept() {
                Ok(_) => return Ok(AsyncReadWrite(TlsStream { io })),
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    interest = Interest::READABLE;
                }
                Err(ref e) if e.code() == ErrorCode::WANT_WRITE => {
                    interest = Interest::WRITABLE;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl<Io: AsyncIo> Service<Io> for TlsAcceptorService {
    type Response = AsyncReadWrite<TlsStream<Io>>;
    type Error = OpensslError;

    async fn call(&self, io: Io) -> Result<Self::Response, Self::Error> {
        self.accept(io).await
    }
}

impl<Io: AsyncIo> AsyncIo for TlsStream<Io> {
    #[inline]
    fn ready(&self, interest: Interest) -> impl Future<Output = io::Result<Ready>> + Send {
        self.io.get_ref().ready(interest)
    }

    #[inline]
    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        self.io.get_ref().poll_ready(interest, cx)
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
                return Poll::Ready(Err(e
                    .into_io_error()
                    .unwrap_or_else(|e| io::Error::new(io::ErrorKind::Other, e))));
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
pub enum OpensslError {
    Io(io::Error),
    Tls(Error),
    Stack(ErrorStack),
}

impl Debug for OpensslError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => Debug::fmt(e, f),
            Self::Tls(ref e) => Debug::fmt(e, f),
            Self::Stack(ref e) => Debug::fmt(e, f),
        }
    }
}

impl From<io::Error> for OpensslError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ErrorStack> for OpensslError {
    fn from(e: ErrorStack) -> Self {
        Self::Stack(e)
    }
}

impl From<Error> for OpensslError {
    fn from(e: Error) -> Self {
        Self::Tls(e)
    }
}

impl From<OpensslError> for TlsError {
    fn from(e: OpensslError) -> Self {
        Self::Openssl(e)
    }
}
