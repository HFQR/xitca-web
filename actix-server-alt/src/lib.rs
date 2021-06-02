//! Multi-threaded server for Tcp/Udp/UnixDomain handling.

#![forbid(unsafe_code)]
#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

mod builder;
mod server;
mod worker;

#[cfg(feature = "signal")]
mod signals;

pub mod net;

pub use builder::Builder;
pub use server::{ServerFuture, ServerHandle};
