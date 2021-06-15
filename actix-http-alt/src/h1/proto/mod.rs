//! protocol module of Http/1.x
//! aiming to be correct and fast with only safe code.

mod buf;
mod codec;
mod context;
mod decode;
mod dispatcher;
mod encode;
mod error;

pub(crate) use dispatcher::Dispatcher;
pub(crate) use error::ProtoError;
