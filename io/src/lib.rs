//! Async traits and types used for Io operations.

#![feature(type_alias_impl_trait)]

pub mod bytes;
#[cfg(feature = "runtime")]
pub mod io;
#[cfg(feature = "runtime")]
pub mod net;
