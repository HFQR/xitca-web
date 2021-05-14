#[cfg(feature = "openssl")]
pub(crate) mod openssl;

pub(crate) mod rustls;

use std::{
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::service::{Service, ServiceFactory};

use super::error::HttpServiceError;

/// A NoOp Tls Acceptor pass through input Stream type.
pub struct NoOpTlsAcceptorFactory;

pub struct NoOpTlsAcceptor<St> {
    _stream: PhantomData<St>,
}

impl<St> ServiceFactory<St> for NoOpTlsAcceptorFactory
where
    St: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = St;
    type Error = HttpServiceError;
    type Config = ();
    type Service = NoOpTlsAcceptor<St>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async move {
            Ok(NoOpTlsAcceptor {
                _stream: PhantomData,
            })
        }
    }
}

impl<St> Service for NoOpTlsAcceptor<St>
where
    St: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Request<'r> = St;
    type Response = St;
    type Error = HttpServiceError;

    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    #[inline]
    fn poll_ready(&self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call<'s, 'r, 'f>(&'s self, io: Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
        async move { Ok(io) }
    }
}
