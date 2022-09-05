//! Multi-threaded server for Tcp/Udp/UnixDomain handling.

#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

mod builder;
mod server;
mod worker;

// TODO: wasm does not support system signal yet.
#[cfg(not(target_family = "wasm"))]
mod signals;

pub mod net;

pub use builder::Builder;
pub use server::{ServerFuture, ServerHandle};

#[cfg(all(not(target_os = "linux"), feature = "io-uring"))]
compile_error!("io_uring can only be used on linux system");
