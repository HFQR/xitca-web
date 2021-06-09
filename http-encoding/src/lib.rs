//! Content-Encoding support on top of `http` crate

#![feature(min_type_alias_impl_trait)]

mod decoder;
mod encoder;

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
mod writer;

#[cfg(feature = "br")]
mod brotli;
#[cfg(feature = "de")]
mod deflate;
#[cfg(feature = "gz")]
mod gzip;

pub use self::decoder::{AsyncDecode, DecodeError, Decoder};
