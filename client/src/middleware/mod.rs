//! middleware offer extended functionality to http client.

mod default_headers;
mod redirect;

#[cfg(feature = "compress")]
mod decompress;

#[cfg(feature = "compress")]
pub use decompress::Decompress;

pub use default_headers::{DefaultHeaderMap, DefaultHeaders};
pub use redirect::FollowRedirect;
