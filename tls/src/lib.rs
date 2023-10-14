#![feature(async_fn_in_trait, return_position_impl_trait_in_trait)]

#[cfg(feature = "rustls")]
pub mod rustls;
#[cfg(feature = "rustls-uring")]
pub mod rustls_uring;
