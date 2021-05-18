use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
    io,
    sync::Arc,
    task::{Context, Poll},
};

use actix_service_alt::{Service, ServiceFactory};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;

use crate::error::HttpServiceError;

pub(crate) use tokio_rustls::{rustls::ServerConfig, server::TlsStream};

/// Rustls Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
#[derive(Clone)]
pub struct TlsAcceptorService {
    acceptor: TlsAcceptor,
}

impl TlsAcceptorService {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self {
            acceptor: TlsAcceptor::from(config),
        }
    }
}

impl<St> ServiceFactory<St> for TlsAcceptorService
where
    St: AsyncRead + AsyncWrite + Unpin,
{
    type Response = TlsStream<St>;
    type Error = RustlsError;
    type Config = ();
    type Service = TlsAcceptorService;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let this = self.clone();
        async move { Ok(this) }
    }
}

impl<St> Service<St> for TlsAcceptorService
where
    St: AsyncRead + AsyncWrite + Unpin,
{
    type Response = TlsStream<St>;
    type Error = RustlsError;

    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'c>(&'c self, io: St) -> Self::Future<'c>
    where
        St: 'c,
    {
        async move {
            let stream = self.acceptor.accept(io).await?;
            Ok(stream)
        }
    }
}

/// Collection of 'rustls' error types.
pub struct RustlsError(io::Error);

impl Debug for RustlsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl From<io::Error> for RustlsError {
    fn from(e: io::Error) -> Self {
        Self(e)
    }
}

impl From<RustlsError> for HttpServiceError {
    fn from(e: RustlsError) -> Self {
        Self::Rustls(e)
    }
}
