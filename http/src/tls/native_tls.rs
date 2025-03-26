pub(crate) use native_tls::TlsAcceptor;

use core::{
    convert::Infallible,
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

use native_tls::{Error, HandshakeError};
use xitca_io::io::{AsyncIo, Interest, Ready};
use xitca_service::Service;

use crate::{http::Version, version::AsVersion};

use super::{error::TlsError, IsTls};

/// A wrapper type for [TlsStream](native_tls::TlsStream).
///
/// This is to impl new trait for it.
pub struct TlsStream<Io> {
    io: native_tls::TlsStream<Io>,
}

impl<Io: AsyncIo> AsVersion for TlsStream<Io> {
    fn as_version(&self) -> Version {
        self.io
            .negotiated_alpn()
            .ok()
            .and_then(|proto| proto)
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

/// native-tls Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
pub struct TlsAcceptorService {
    acceptor: TlsAcceptor,
}

impl<St: AsyncIo> Service<St> for TlsAcceptorService {
    type Response = TlsStream<St>;
    type Error = NativeTlsError;

    async fn call(&self, mut io: St) -> Result<Self::Response, Self::Error> {
        let mut interest = Interest::READABLE;

        io.ready(interest).await?;

        let mut res = self.acceptor.accept(io);

        loop {
            let mut stream = match res {
                Ok(io) => return Ok(TlsStream { io }),
                Err(HandshakeError::WouldBlock(stream)) => {
                    interest = Interest::READABLE;
                    stream
                }
                Err(HandshakeError::Failure(e)) => return Err(e.into()),
            };

            stream.get_mut().ready(interest).await?;

            res = stream.handshake();
        }
    }
}

impl IsTls for TlsAcceptorService {}

impl<S: AsyncIo> AsyncIo for TlsStream<S> {
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

        this.io.shutdown()?;

        AsyncIo::poll_shutdown(Pin::new(this.io.get_mut()), cx)
    }
}

impl<S: AsyncIo> io::Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.io, buf)
    }
}

impl<S: AsyncIo> io::Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(&mut self.io, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.io)
    }
}

/// Collection of 'native-tls' error types.
pub enum NativeTlsError {
    Io(io::Error),
    Tls(Error),
}

impl fmt::Debug for NativeTlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => fmt::Debug::fmt(e, f),
            Self::Tls(ref e) => fmt::Debug::fmt(e, f),
        }
    }
}

impl fmt::Display for NativeTlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => fmt::Display::fmt(e, f),
            Self::Tls(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl From<io::Error> for NativeTlsError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Error> for NativeTlsError {
    fn from(e: Error) -> Self {
        Self::Tls(e)
    }
}

impl From<NativeTlsError> for TlsError {
    fn from(e: NativeTlsError) -> Self {
        Self::NativeTls(e)
    }
}
