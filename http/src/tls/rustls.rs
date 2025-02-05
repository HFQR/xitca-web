use core::{convert::Infallible, fmt};

use std::{error, io, sync::Arc};

use xitca_io::io::AsyncIo;
use xitca_service::Service;
use xitca_tls::rustls::{Error, ServerConfig, ServerConnection, TlsStream as _TlsStream};

use crate::{http::Version, version::AsVersion};

use super::{error::TlsError, IsTls};

pub(crate) type RustlsConfig = Arc<ServerConfig>;

/// A stream managed by rustls for tls read/write.
pub type TlsStream<Io> = _TlsStream<ServerConnection, Io>;

impl<Io> AsVersion for TlsStream<Io>
where
    Io: AsyncIo,
{
    fn as_version(&self) -> Version {
        self.session()
            .alpn_protocol()
            .map(Self::from_alpn)
            .unwrap_or(Version::HTTP_11)
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

impl<Io: AsyncIo> Service<Io> for TlsAcceptorService {
    type Response = TlsStream<Io>;
    type Error = RustlsError;

    async fn call(&self, io: Io) -> Result<Self::Response, Self::Error> {
        let conn = ServerConnection::new(self.acceptor.clone())?;
        _TlsStream::handshake(io, conn).await.map_err(Into::into)
    }
}

impl IsTls for TlsAcceptorService {}

/// Collection of 'rustls' error types.
pub enum RustlsError {
    Io(io::Error),
    Tls(Error),
}

impl fmt::Debug for RustlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => fmt::Debug::fmt(e, f),
            Self::Tls(ref e) => fmt::Debug::fmt(e, f),
        }
    }
}

impl fmt::Display for RustlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => fmt::Display::fmt(e, f),
            Self::Tls(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl error::Error for RustlsError {}

impl From<io::Error> for RustlsError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Error> for RustlsError {
    fn from(e: Error) -> Self {
        Self::Tls(e)
    }
}

impl From<RustlsError> for TlsError {
    fn from(e: RustlsError) -> Self {
        Self::Rustls(e)
    }
}
