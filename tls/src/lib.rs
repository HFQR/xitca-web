#[cfg(feature = "native-tls")]
pub mod native_tls_complete;
#[cfg(feature = "openssl")]
pub mod openssl;
#[cfg(feature = "openssl")]
pub mod openssl_complete;
#[cfg(any(feature = "rustls", feature = "rustls-ring-crypto", feature = "rustls-aws-crypto"))]
pub mod rustls;
#[cfg(feature = "rustls")]
pub mod rustls_complete;

#[cfg(any(feature = "openssl", feature = "native-tls"))]
pub(crate) mod bridge;
