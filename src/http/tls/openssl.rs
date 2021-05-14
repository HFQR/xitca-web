use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use openssl::error::{Error, ErrorStack};
use openssl::ssl::{Error as TlsError, Ssl};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::http::error::HttpServiceError;
use crate::service::{Service, ServiceFactory};

pub(crate) use openssl::ssl::SslAcceptor as TlsAcceptor;
pub(crate) use tokio_openssl::SslStream as TlsStream;

/// Openssl Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
pub struct TlsAcceptorService<St> {
    acceptor: TlsAcceptor,
    _stream: PhantomData<St>,
}

impl<St> TlsAcceptorService<St> {
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Self {
            acceptor,
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
    type Error = OpensslError;
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
    type Error = OpensslError;

    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    #[inline]
    fn poll_ready(&self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'s, 'r, 'f>(&'s self, io: Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
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
