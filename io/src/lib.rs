//! Async traits and types used for Io operations.

#![forbid(unsafe_code)]
#![feature(async_fn_in_trait, return_position_impl_trait_in_trait)]

pub mod bytes;
#[cfg(feature = "runtime")]
pub mod io;
#[cfg(feature = "runtime-uring")]
pub mod io_uring;
#[cfg(feature = "runtime")]
pub mod net;
