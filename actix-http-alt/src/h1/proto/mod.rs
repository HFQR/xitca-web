//! protocol module of Http/1.x
//! aiming to be correct and fast with only safe code.

mod context;
mod decode;
mod dispatcher;
mod encode;
mod error;
mod state;

pub(super) use dispatcher::Dispatcher;
pub(super) use error::ProtoError;
