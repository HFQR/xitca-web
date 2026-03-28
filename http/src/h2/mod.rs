//! http/2 specific module for types and protocol utilities.

mod builder;
mod dispatcher;
mod error;
mod proto;
mod service;

pub mod body;
#[cfg(feature = "io-uring")]
pub mod dispatcher_uring;

pub use self::body::RequestBody;
pub use self::error::Error;
pub use self::service::H2Service;

pub(crate) use dispatcher::Dispatcher;
