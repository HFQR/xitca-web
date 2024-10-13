//! http/1 specific module for types and protocol utilities.

pub mod dispatcher_unreal;
pub mod proto;

pub(crate) mod dispatcher;

mod body;
mod builder;
mod error;
mod service;

pub use self::body::RequestBody;
pub use self::error::Error;
pub use self::service::H1Service;

#[cfg(feature = "io-uring")]
mod dispatcher_uring;
