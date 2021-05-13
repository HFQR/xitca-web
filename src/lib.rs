#![forbid(unsafe_code)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

mod builder;
mod server;
mod service;
mod worker;

#[cfg(feature = "signal")]
mod signals;

pub mod http;
pub mod net;

pub use builder::Builder;
pub use server::{ServerFuture, ServerHandle};
pub use service::{Service, ServiceFactory};
