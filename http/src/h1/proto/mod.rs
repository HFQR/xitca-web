//! protocol module of Http/1.x
//! aiming to be correct and fast with only safe code.

mod decode;
mod encode;

pub mod buf_write;
pub mod codec;
pub mod context;
pub mod error;
pub mod header;
