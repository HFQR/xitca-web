use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_service_alt::{Service, ServiceFactory};
use openssl_crate::error::{Error, ErrorStack};
use openssl_crate::ssl::{Error as TlsError, Ssl};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::HttpServiceError;

pub(crate) use openssl_crate::ssl::SslAcceptor as TlsAcceptor;
pub(crate) use tokio_openssl::SslStream as TlsStream;

/// Openssl Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
#[derive(Clone)]
pub struct TlsAcceptorService {
    acceptor: TlsAcceptor,
}

impl TlsAcceptorService {
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Self { acceptor }
    }
}

impl<St> ServiceFactory<St> for TlsAcceptorService
where
    St: AsyncRead + AsyncWrite + Unpin,
{
    type Response = TlsStream<St>;
    type Error = OpensslError;
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
    type Error = OpensslError;

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
            let ctx = self.acceptor.context();
            let ssl = Ssl::new(ctx)?;
            let mut stream = TlsStream::new(ssl, io)?;

            Pin::new(&mut stream).accept().await?;

            Ok(stream)
        }
    }
}

/// Collection of 'openssl' error types.
pub enum OpensslError {
    Ssl(TlsError),
    Single(Error),
    Stack(ErrorStack),
}

impl Debug for OpensslError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Ssl(ref e) => write!(f, "{:?}", e),
            Self::Single(ref e) => write!(f, "{:?}", e),
            Self::Stack(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl From<Error> for OpensslError {
    fn from(e: Error) -> Self {
        Self::Single(e)
    }
}

impl From<ErrorStack> for OpensslError {
    fn from(e: ErrorStack) -> Self {
        Self::Stack(e)
    }
}

impl From<TlsError> for OpensslError {
    fn from(e: TlsError) -> Self {
        Self::Ssl(e)
    }
}

impl From<OpensslError> for HttpServiceError {
    fn from(e: OpensslError) -> Self {
        Self::Openssl(e)
    }
}
