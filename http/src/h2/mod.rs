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
pub use self::{body::RequestBodyV2, proto::run};

#[cfg(feature = "io-uring")]
pub(super) use self::body::RequestBodySender;
