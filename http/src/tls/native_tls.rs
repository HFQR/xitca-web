pub(crate) use xitca_tls::native_tls::TlsAcceptor;

use core::convert::Infallible;

use xitca_io::io::{AsyncBufRead, AsyncBufWrite};
use xitca_service::Service;

use crate::{http::Version, version::AsVersion};

use super::error::TlsError;

pub type TlsStream<Io> = xitca_tls::native_tls::TlsStream<Io>;

impl<Io> AsVersion for TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    fn as_version(&self) -> Version {
        self.session()
            .negotiated_alpn()
            .ok()
            .flatten()
            .as_deref()
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

impl<Io> Service<Io> for TlsAcceptorService
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    type Response = TlsStream<Io>;
    type Error = NativeTlsError;

    async fn call(&self, io: Io) -> Result<Self::Response, Self::Error> {
        TlsStream::accept(&self.acceptor, io).await
    }
}

/// Collection of 'native-tls' error types.
pub type NativeTlsError = xitca_tls::native_tls::Error;

impl From<NativeTlsError> for TlsError {
    fn from(e: NativeTlsError) -> Self {
        Self::NativeTls(e)
    }
}
