//! Content-Encoding support on top of `http` crate

#![forbid(unsafe_code)]

#[macro_use]
mod coder;
mod coding;
mod decoder;
mod encoder;

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
mod writer;

#[cfg(feature = "br")]
mod brotli {
    pub(super) use brotli2::write::{BrotliDecoder, BrotliEncoder};
    code_impl!(BrotliDecoder);
    code_impl!(BrotliEncoder);
}
#[cfg(feature = "gz")]
mod gzip {
    pub(super) use flate2::write::{GzDecoder, GzEncoder};
    code_impl!(GzDecoder);
    code_impl!(GzEncoder);
}
#[cfg(feature = "de")]
mod deflate {
    pub(super) use flate2::write::{DeflateDecoder, DeflateEncoder};
    code_impl!(DeflateDecoder);
    code_impl!(DeflateEncoder);
}

pub use self::coder::{Code, Coder};
pub use self::coding::ContentEncoding;
pub use self::decoder::try_decoder;
pub use self::encoder::try_encoder;
