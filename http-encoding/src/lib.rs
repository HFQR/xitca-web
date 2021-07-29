//! Content-Encoding support on top of `http` crate

#![forbid(unsafe_code)]
#![feature(type_alias_impl_trait)]

#[macro_use]
mod coder;
mod coding;
mod decoder;
mod encoder;

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
mod writer;

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
mod r#const {
    pub(super) const MAX_CHUNK_SIZE_DECODE_IN_PLACE: usize = 2049;
    pub(super) const MAX_CHUNK_SIZE_ENCODE_IN_PLACE: usize = 1024;
}

#[cfg(feature = "br")]
mod brotli {
    pub(super) use brotli2::write::{BrotliDecoder, BrotliEncoder};
    async_code_impl!(BrotliDecoder, super::r#const::MAX_CHUNK_SIZE_DECODE_IN_PLACE);
    async_code_impl!(BrotliEncoder, super::r#const::MAX_CHUNK_SIZE_ENCODE_IN_PLACE);
}
#[cfg(feature = "gz")]
mod gzip {
    pub(super) use flate2::write::{GzDecoder, GzEncoder};
    async_code_impl!(GzDecoder, super::r#const::MAX_CHUNK_SIZE_DECODE_IN_PLACE);
    async_code_impl!(GzEncoder, super::r#const::MAX_CHUNK_SIZE_ENCODE_IN_PLACE);
}
#[cfg(feature = "de")]
mod deflate {
    pub(super) use flate2::write::{DeflateDecoder, DeflateEncoder};
    async_code_impl!(DeflateDecoder, super::r#const::MAX_CHUNK_SIZE_DECODE_IN_PLACE);
    async_code_impl!(DeflateEncoder, super::r#const::MAX_CHUNK_SIZE_ENCODE_IN_PLACE);
}

pub use self::coder::{AsyncCode, Coder};
pub use self::coding::ContentEncoding;
