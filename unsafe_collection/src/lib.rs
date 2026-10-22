//! A collection of utility code that use unsafe blocks.
//!
//! Code in this collection is not unsafe by nature. They are here to avoid using more nightly
//! Rust features and/or due to lack of 3rd party crates offer a similar functionality.
//!
//! The crate is `no_std` by default and depends on `core` only. Enable the `alloc` feature for
//! utilities that depend on `alloc` and the `std` feature (implies `alloc`) for utilities that
//! depend on `std`.

#![no_std]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;

#[cfg(any(feature = "std", test))]
extern crate std;

pub mod fake;
pub mod futures;
pub mod no_hash;
pub mod uninit;

#[cfg(feature = "alloc")]
pub mod small_str;

#[cfg(feature = "bytes")]
pub mod bytes;
