//! Async traits and types used for Io operations.

#![deny(unsafe_code)]

pub mod bytes;
pub mod io;

#[cfg(feature = "runtime")]
pub mod net;
