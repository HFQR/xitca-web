//! Async traits and types used for Io operations.

#![forbid(unsafe_code)]
#![feature(impl_trait_in_assoc_type)]

pub mod bytes;
#[cfg(feature = "runtime")]
pub mod io;
#[cfg(feature = "runtime-uring")]
pub mod io_uring;
#[cfg(feature = "runtime")]
pub mod net;
