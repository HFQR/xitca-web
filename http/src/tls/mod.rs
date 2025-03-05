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

use xitca_service::Service;

/// A trait to check if an acceptor will create a Tls stream.
pub trait IsTls {
    fn is_tls(&self) -> bool {
        true
    }
}

/// A NoOp Tls Acceptor pass through input Stream type.
#[derive(Copy, Clone)]
pub struct NoOpTlsAcceptorBuilder;

impl Service for NoOpTlsAcceptorBuilder {
    type Response = NoOpTlsAcceptorService;
    type Error = TlsError;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        Ok(NoOpTlsAcceptorService)
    }
}

pub struct NoOpTlsAcceptorService;

impl<St> Service<St> for NoOpTlsAcceptorService {
    type Response = St;
    type Error = TlsError;

    async fn call(&self, io: St) -> Result<Self::Response, Self::Error> {
        Ok(io)
    }
}

impl IsTls for NoOpTlsAcceptorService {
    fn is_tls(&self) -> bool {
        false
    }
}
