//! protocol module of Http/1.x
//! aiming to be correct and fast with only safe code.

mod buf;
mod context;
mod decode;
mod dispatcher;
mod encode;
mod error;
mod keep_alive;
mod state;

pub(super) use dispatcher::Dispatcher;
pub(super) use error::ProtoError;
pub(super) use keep_alive::KeepAlive;
