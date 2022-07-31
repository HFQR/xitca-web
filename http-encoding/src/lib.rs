//! Content-Encoding support on top of `http` crate

#![forbid(unsafe_code)]

pub mod error;

#[macro_use]
mod coder;
mod coding;
mod decode;
mod encode;

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
mod writer;

#[cfg(feature = "br")]
mod brotli {
    use std::io::{self, Write};

    use brotli2::write::{BrotliDecoder, BrotliEncoder};
    use bytes::Bytes;

    use super::{coder::Code, writer::Writer};

    pub type Decoder = BrotliDecoder<Writer>;
    pub struct Encoder(Option<BrotliEncoder<Writer>>);

    impl Encoder {
        pub(crate) fn new(level: u32) -> Self {
            Self(Some(BrotliEncoder::new(Writer::new(), level)))
        }
    }

    impl<T> Code<T> for BrotliDecoder<Writer>
    where
        T: AsRef<[u8]>,
    {
        type Item = Bytes;

        fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
            self.write_all(item.as_ref())?;
            self.flush()?;
            let b = self.get_mut().take();
            if !b.is_empty() {
                Ok(Some(b))
            } else {
                Ok(None)
            }
        }

        fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
            let b = self.finish()?.take_owned();
            if !b.is_empty() {
                Ok(Some(b))
            } else {
                Ok(None)
            }
        }
    }

    impl<T> Code<T> for Encoder
    where
        T: AsRef<[u8]>,
    {
        type Item = Bytes;

        fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
            let encoder = self.0.as_mut().unwrap();
            encoder.write_all(item.as_ref())?;
            encoder.flush()?;
            let b = encoder.get_mut().take();
            if !b.is_empty() {
                Ok(Some(b))
            } else {
                Ok(None)
            }
        }

        fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
            match self.0.take() {
                Some(encoder) => {
                    let b = encoder.finish()?.take_owned();
                    assert!(!b.is_empty());
                    Ok(Some(b))
                }
                None => Ok(None),
            }
        }
    }
}

#[cfg(feature = "gz")]
mod gzip {
    use super::writer::Writer;

    use flate2::write::{GzDecoder, GzEncoder};

    pub type Decoder = GzDecoder<Writer>;
    pub type Encoder = GzEncoder<Writer>;

    code_impl!(GzDecoder);
    code_impl!(GzEncoder);
}
#[cfg(feature = "de")]
mod deflate {
    use super::writer::Writer;

    use flate2::write::{DeflateDecoder, DeflateEncoder};

    pub type Decoder = DeflateDecoder<Writer>;
    pub type Encoder = DeflateEncoder<Writer>;

    code_impl!(DeflateDecoder);
    code_impl!(DeflateEncoder);
}

pub use self::coder::{Code, Coder, FeaturedCode};
pub use self::coding::ContentEncoding;
pub use self::decode::try_decoder;
pub use self::encode::encoder;
