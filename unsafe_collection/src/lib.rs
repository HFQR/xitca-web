//! A collection of utility code that use unsafe blocks.
//!
//! Code in this collection is not unsafe by nature. They are here to avoid using more nightly
//! Rust features and/or due to lack of 3rd party crates offer a similar functionality.

pub mod bound_queue;
pub mod fake_send_sync;
pub mod futures;
pub mod no_hash;
pub mod uninit;

#[cfg(feature = "bytes")]
pub mod bytes;
#[cfg(feature = "channel")]
pub mod channel;

#[cfg(feature = "channel")]
mod list;
