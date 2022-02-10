//! xitca-http treat all connection as running on top of tls and would try to do tls
//! accept on all of them.
//!
//! For plain Tcp and Unix sockets connection a dummy Tls acceptor and tls stream type
//! is used.

#[cfg(feature = "native-tls")]
pub(crate) mod native_tls;
#[cfg(feature = "openssl")]
pub(crate) mod openssl;
#[cfg(feature = "rustls")]
pub(crate) mod rustls;

mod error;

pub(crate) use error::TlsError;

use std::future::Future;

use xitca_service::{Service, ServiceFactory};

/// A NoOp Tls Acceptor pass through input Stream type.
#[derive(Copy, Clone)]
pub struct NoOpTlsAcceptorService;

impl<St, Arg> ServiceFactory<St, Arg> for NoOpTlsAcceptorService {
    type Response = St;
    type Error = TlsError;
    type Service = Self;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, _: Arg) -> Self::Future {
        async { Ok(Self) }
    }
}

impl<St> Service<St> for NoOpTlsAcceptorService {
    type Response = St;
    type Error = TlsError;

    type Ready<'f> = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline(always)]
    fn ready(&self) -> Self::Ready<'_> {
        async { Ok(()) }
    }

    #[inline(always)]
    fn call(&self, io: St) -> Self::Future<'_> {
        async move { Ok(io) }
    }
}
