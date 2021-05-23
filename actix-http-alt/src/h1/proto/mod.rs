//! protocol module of Http/1.x
//! aiming to be correct and fast with only safe code.

mod context;
mod dispatcher;
mod error;
mod state;

pub(super) use dispatcher::Dispatcher;
