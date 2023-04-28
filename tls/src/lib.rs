#![feature(impl_trait_in_assoc_type)]

#[cfg(feature = "rustls")]
pub mod rustls;
#[cfg(feature = "rustls-uring")]
pub mod rustls_uring;
