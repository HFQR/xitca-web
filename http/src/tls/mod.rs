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
#[cfg(feature = "rustls-uring")]
pub(crate) mod rustls_uring;

mod error;

pub use error::TlsError;

use std::future::Future;

use xitca_service::Service;

/// A NoOp Tls Acceptor pass through input Stream type.
#[derive(Copy, Clone)]
pub struct NoOpTlsAcceptorBuilder;

impl Service for NoOpTlsAcceptorBuilder {
    type Response = NoOpTlsAcceptorService;
    type Error = TlsError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call<'s>(&self, _: ()) -> Self::Future<'s> {
        async { Ok(NoOpTlsAcceptorService) }
    }
}

pub struct NoOpTlsAcceptorService;

impl<St> Service<St> for NoOpTlsAcceptorService {
    type Response = St;
    type Error = TlsError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where St: 'f;

    #[inline(always)]
    fn call<'s>(&self, io: St) -> Self::Future<'s> {
        async { Ok(io) }
    }
}
