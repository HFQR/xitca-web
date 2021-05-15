use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
    io,
    marker::PhantomData,
    sync::Arc,
    task::{Context, Poll},
};

use actix_service_alt::{Service, ServiceFactory};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;

use crate::error::HttpServiceError;

pub(crate) use tokio_rustls::{rustls::ServerConfig, server::TlsStream};

/// Rustls Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
pub struct TlsAcceptorService<St> {
    acceptor: TlsAcceptor,
    _stream: PhantomData<St>,
}

impl<St> TlsAcceptorService<St> {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self {
            acceptor: TlsAcceptor::from(config),
            _stream: PhantomData,
        }
    }
}

impl<St> Clone for TlsAcceptorService<St> {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            _stream: PhantomData,
        }
    }
}

impl<St> ServiceFactory<St> for TlsAcceptorService<St>
where
    St: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = TlsStream<St>;
    type Error = RustlsError;
    type Config = ();
    type Service = TlsAcceptorService<St>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let this = self.clone();
        async move { Ok(this) }
    }
}

impl<St> Service for TlsAcceptorService<St>
where
    St: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Request<'r> = St;
    type Response = TlsStream<St>;
    type Error = RustlsError;

    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    #[inline]
    fn poll_ready(&self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'s>(&'s self, io: Self::Request<'s>) -> Self::Future<'s> {
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
