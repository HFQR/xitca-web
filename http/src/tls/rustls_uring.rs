use core::convert::Infallible;

use std::{io, net::Shutdown, sync::Arc};

use xitca_io::io_uring::{AsyncBufRead, AsyncBufWrite, BoundedBuf, BoundedBufMut};
use xitca_service::Service;
use xitca_tls::{
    rustls::{ServerConfig, ServerConnection},
    rustls_uring::TlsStream as _TlsStream,
};

use crate::{http::Version, version::AsVersion};

use super::{rustls::RustlsError, IsTls};

/// A stream managed by rustls for tls read/write.
pub struct TlsStream<Io> {
    inner: _TlsStream<ServerConnection, Io>,
}

impl<Io> AsVersion for TlsStream<Io> {
    fn as_version(&self) -> Version {
        Version::HTTP_11
    }
}

#[derive(Clone)]
pub struct TlsAcceptorBuilder {
    acceptor: Arc<ServerConfig>,
}

impl TlsAcceptorBuilder {
    pub fn new(acceptor: Arc<ServerConfig>) -> Self {
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

/// Rustls Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
pub struct TlsAcceptorService {
    acceptor: Arc<ServerConfig>,
}

impl<Io> Service<Io> for TlsAcceptorService
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    type Response = TlsStream<Io>;
    type Error = RustlsError;

    async fn call(&self, io: Io) -> Result<Self::Response, Self::Error> {
        let conn = ServerConnection::new(self.acceptor.clone())?;
        let inner = _TlsStream::handshake(io, conn).await?;
        Ok(TlsStream { inner })
    }
}

impl IsTls for TlsAcceptorService {}

impl<Io> AsyncBufRead for TlsStream<Io>
where
    Io: AsyncBufRead,
{
    #[inline]
    async fn read<B>(&self, buf: B) -> (io::Result<usize>, B)
    where
        B: BoundedBufMut,
    {
        self.inner.read(buf).await
    }
}

impl<Io> AsyncBufWrite for TlsStream<Io>
where
    Io: AsyncBufWrite,
{
    #[inline]
    async fn write<B>(&self, buf: B) -> (io::Result<usize>, B)
    where
        B: BoundedBuf,
    {
        self.inner.write(buf).await
    }

    fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
        self.inner.shutdown(direction)
    }
}
