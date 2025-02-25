//! middleware offer extended functionality to http client.

mod redirect;

mod async_fn;
#[cfg(feature = "compress")]
mod decompress;

#[cfg(feature = "compress")]
pub use decompress::Decompress;

pub(crate) use async_fn::AsyncFn;
pub use redirect::FollowRedirect;
