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
    use std::io::{self, Write};

    use bytes::Bytes;

    use crate::{coder::Code, writer::Writer};

    pub(super) use brotli2::write::{BrotliDecoder, BrotliEncoder};

    impl<T> Code<T> for BrotliDecoder<Writer>
    where
        T: AsRef<[u8]>,
    {
        type Item = Bytes;

        fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
            self.write_all(item.as_ref())?;
            let b = self.get_mut().take();
            if !b.is_empty() {
                Ok(Some(b))
            } else {
                Ok(None)
            }
        }

        fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
            self.finish()?;
            let b = self.get_mut().take();
            if !b.is_empty() {
                Ok(Some(b))
            } else {
                Ok(None)
            }
        }
    }

    impl<T> Code<T> for BrotliEncoder<Writer>
    where
        T: AsRef<[u8]>,
    {
        type Item = Bytes;

        fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
            self.write_all(item.as_ref())?;
            let b = self.get_mut().take();
            if !b.is_empty() {
                Ok(Some(b))
            } else {
                Ok(None)
            }
        }

        fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
            self.flush()?;
            let b = self.get_mut().take();
            if !b.is_empty() {
                Ok(Some(b))
            } else {
                Ok(None)
            }
        }
    }
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
pub use self::decoder::{try_decoder, ContentDecoder};
pub use self::encoder::{try_encoder, ContentEncoder};
