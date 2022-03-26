//! A collection of utility code that use unsafe blocks.
//!
//! Code in this collection is not unsafe by nature. They are here to avoid using more nightly
//! Rust features and/or due to lack of 3rd party crates offer a similar functionality.

#![no_std]

pub mod array_queue;
pub mod uninit;
