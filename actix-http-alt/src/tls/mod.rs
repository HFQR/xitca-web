#[cfg(feature = "rustls")]
pub(crate) mod rustls;

use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_server_alt::net::{AsyncReadWrite, Protocol};
use actix_service_alt::{Service, ServiceFactory};

use super::error::HttpServiceError;

/// A NoOp Tls Acceptor pass through input Stream type.
pub struct NoOpTlsAcceptorFactory;

impl<St: AsyncReadWrite> ServiceFactory<St> for NoOpTlsAcceptorFactory {
    type Response = (St, Protocol);
    type Error = HttpServiceError;
    type Config = ();
    type Service = NoOpTlsAcceptor;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async move { Ok(NoOpTlsAcceptor) }
    }
}

pub struct NoOpTlsAcceptor;

impl<St: AsyncReadWrite> Service<St> for NoOpTlsAcceptor {
    type Response = (St, Protocol);
    type Error = HttpServiceError;

    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call<'c>(&'c self, io: St) -> Self::Future<'c>
    where
        St: 'c,
    {
        async move { Ok((io, Protocol::Http1)) }
    }
}
