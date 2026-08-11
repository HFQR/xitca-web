#![allow(dead_code)]

mod exclusive;
mod shared;

// pluggable pool service layer.
pub(crate) mod service;

// load balancing pool implementation built on top of the exclusive / shared pools.
pub(crate) mod balance;
