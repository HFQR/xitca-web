#![forbid(unsafe_code)]

extern crate alloc;

mod error;
mod params;
mod router;
mod tree;

pub use error::{InsertError, MatchError};
pub use params::{Params, ParamsIntoIter};
pub use router::{Match, Router};

pub use xitca_unsafe_collection::bytes::BytesStr;
