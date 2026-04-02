#[cfg(feature = "native-tls")]
pub mod native_tls;

#[cfg(feature = "openssl")]
pub mod openssl;
#[cfg(feature = "openssl-poll")]
pub mod openssl_poll;

#[cfg(feature = "rustls")]
pub mod rustls;
#[cfg(any(feature = "rustls-poll-ring-crypto", feature = "rustls-poll-aws-crypto"))]
pub mod rustls_poll;

#[cfg(any(feature = "openssl", feature = "native-tls"))]
pub(crate) mod bridge;
