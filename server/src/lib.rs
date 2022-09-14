//! Multi-threaded server for Tcp/Udp/UnixDomain handling.

#![forbid(unsafe_code)]
#![feature(type_alias_impl_trait)]

mod builder;
mod server;
mod signals;
mod worker;

pub mod net;

pub use builder::Builder;
pub use server::{ServerFuture, ServerHandle};

#[cfg(all(not(target_os = "linux"), feature = "io-uring"))]
compile_error!("io_uring can only be used on linux system");
