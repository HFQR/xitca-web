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

pub use error::TlsError;

use std::future::Future;

use xitca_service::{BuildService, Service};

/// A NoOp Tls Acceptor pass through input Stream type.
#[derive(Copy, Clone)]
pub struct NoOpTlsAcceptorService;

impl BuildService for NoOpTlsAcceptorService {
    type Service = Self;
    type Error = TlsError;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, _: ()) -> Self::Future {
        async { Ok(Self) }
    }
}

impl<St> Service<St> for NoOpTlsAcceptorService {
    type Response = St;
    type Error = TlsError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline(always)]
    fn call(&self, io: St) -> Self::Future<'_> {
        async { Ok(io) }
    }
}
