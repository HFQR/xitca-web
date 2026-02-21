pub(crate) use xitca_tls::openssl::ssl::SslAcceptor as TlsAcceptor;

use core::convert::Infallible;

use xitca_io::io::AsyncIo;
use xitca_service::Service;
use xitca_tls::openssl::ssl;

use crate::{http::Version, version::AsVersion};

use super::{IsTls, error::TlsError};

pub type TlsStream<Io> = xitca_tls::openssl::TlsStream<Io>;

impl<Io> AsVersion for TlsStream<Io>
where
    Io: AsyncIo,
{
    fn as_version(&self) -> Version {
        self.session()
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
    async fn accept<Io: AsyncIo>(&self, io: Io) -> Result<TlsStream<Io>, OpensslError> {
        let ctx = self.acceptor.context();
        let ssl = ssl::Ssl::new(ctx)?;
        TlsStream::accept(ssl, io).await
    }
}

impl<Io: AsyncIo> Service<Io> for TlsAcceptorService {
    type Response = TlsStream<Io>;
    type Error = OpensslError;

    async fn call(&self, io: Io) -> Result<Self::Response, Self::Error> {
        self.accept(io).await
    }
}

impl IsTls for TlsAcceptorService {}

/// Collection of 'openssl' error types.
pub type OpensslError = xitca_tls::openssl::Error;

impl From<OpensslError> for TlsError {
    fn from(e: OpensslError) -> Self {
        TlsError::Openssl(e)
    }
}
