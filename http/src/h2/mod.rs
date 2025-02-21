//! http/2 specific module for types and protocol utilities.

mod builder;
mod error;
mod proto;
mod service;

pub mod body;

pub(crate) use self::proto::Dispatcher;

pub use self::body::RequestBody;
pub use self::error::Error;
pub use self::service::H2Service;

#[cfg(feature = "io-uring")]
pub use self::proto::{RequestBody as RequestBodyV2, RequestBodySender, run};
