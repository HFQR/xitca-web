use core::{convert::Infallible, future::Future};

use std::{io, net::Shutdown, sync::Arc};

use rustls::{ServerConfig, ServerConnection};
use xitca_io::io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf, IoBufMut};
use xitca_service::Service;
use xitca_tls::rustls_uring::TlsStream as _TlsStream;

use crate::{http::Version, version::AsVersion};

use super::rustls::RustlsError;

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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call<'s>(&self, _: ()) -> Self::Future<'s> {
        let service = TlsAcceptorService {
            acceptor: self.acceptor.clone(),
        };
        async { Ok(service) }
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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Io: 'f;

    fn call<'s>(&'s self, io: Io) -> Self::Future<'s>
    where
        Io: 's,
    {
        async move {
            let conn = ServerConnection::new(self.acceptor.clone())?;
            let inner = _TlsStream::handshake(io, conn).await?;
            Ok(TlsStream { inner })
        }
    }
}

impl<Io> AsyncBufRead for TlsStream<Io>
where
    Io: AsyncBufRead,
{
    type Future<'f, B> = impl Future<Output=(io::Result<usize>, B)> + 'f where Self: 'f, B: IoBufMut + 'f;

    #[inline]
    fn read<B>(&self, buf: B) -> Self::Future<'_, B>
    where
        B: IoBufMut,
    {
        self.inner.read(buf)
    }
}

impl<Io> AsyncBufWrite for TlsStream<Io>
where
    Io: AsyncBufWrite,
{
    type Future<'f, B> = impl Future<Output=(io::Result<usize>, B)> + 'f where Self: 'f, B: IoBuf + 'f;

    #[inline]
    fn write<B>(&self, buf: B) -> Self::Future<'_, B>
    where
        B: IoBuf,
    {
        self.inner.write(buf)
    }

    fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
        self.inner.shutdown(direction)
    }
}
