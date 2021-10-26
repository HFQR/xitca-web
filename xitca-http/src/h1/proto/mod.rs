//! protocol module of Http/1.x
//! aiming to be correct and fast with only safe code.

mod buf;
mod decode;
mod dispatcher;
mod encode;

pub mod codec;
pub mod context;
pub mod error;
pub mod header;

pub(crate) use dispatcher::run;
