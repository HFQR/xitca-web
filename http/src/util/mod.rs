//! utility module for useful middleware and service types.

pub mod middleware;
pub mod service;

pub(crate) mod futures;
#[cfg(feature = "runtime")]
pub(crate) mod shutdown;
#[cfg(feature = "runtime")]
pub(crate) mod timer;

#[cfg(feature = "runtime")]
pub use shutdown::{ShutdownHandle, ShutdownToken};
