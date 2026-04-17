//! http/2 specific module for types and protocol utilities.

mod body;
mod builder;
mod error;
mod proto;
mod service;
mod util;

pub(crate) mod dispatcher;

pub use self::body::RequestBody;
pub use self::error::Error;
pub use self::service::H2Service;

const STREAM_MUST_EXIST: &str = "Stream MUST NOT be removed while RequestBody or StreamGuard is still alive";
