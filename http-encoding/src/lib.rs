//! Content-Encoding support on top of `http` crate

#![forbid(unsafe_code)]

pub mod error;

#[macro_use]
mod coder;
mod coding;
mod decode;

#[cfg(any(feature = "br", feature = "gz", feature = "de", feature = "zs"))]
mod writer;

#[cfg(feature = "br")]
mod brotli {
    use std::io::{self, Write};

    use brotli2::write::{BrotliDecoder, BrotliEncoder};
    use bytes::Bytes;

    use super::{coder::Code, writer::BytesMutWriter};

    pub struct Decoder(Option<BrotliDecoder<BytesMutWriter>>);

    impl Decoder {
        pub(crate) fn new() -> Self {
            Self(Some(BrotliDecoder::new(BytesMutWriter::new())))
        }
    }

    pub struct Encoder(Option<BrotliEncoder<BytesMutWriter>>);

    impl Encoder {
        pub(crate) fn new(level: u32) -> Self {
            Self(Some(BrotliEncoder::new(BytesMutWriter::new(), level)))
        }
    }

    impl<T> Code<T> for Decoder
    where
        T: AsRef<[u8]>,
    {
        type Item = Bytes;

        fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
            let decoder = self.0.as_mut().unwrap();
            decoder.write_all(item.as_ref())?;
            decoder.flush()?;
            let b = decoder.get_mut().take();
            if !b.is_empty() { Ok(Some(b)) } else { Ok(None) }
        }

        fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
            match self.0.take() {
                Some(mut decoder) => {
                    let b = decoder.finish()?.take_owned();
                    Ok(Some(b))
                }
                None => Ok(None),
            }
        }

        #[inline]
        fn is_end_stream(&self, _: &impl http_body_alt::Body) -> bool {
            self.0.is_none()
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
            if !b.is_empty() { Ok(Some(b)) } else { Ok(None) }
        }

        fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
            match self.0.take() {
                Some(encoder) => {
                    let b = encoder.finish()?.take_owned();
                    Ok(Some(b))
                }
                None => Ok(None),
            }
        }

        #[inline]
        fn is_end_stream(&self, _: &impl http_body_alt::Body) -> bool {
            self.0.is_none()
        }
    }
}

#[cfg(feature = "gz")]
mod gzip {
    use super::writer::BytesMutWriter;

    use flate2::write::{GzDecoder, GzEncoder};

    pub type Decoder = GzDecoder<BytesMutWriter>;
    pub type Encoder = GzEncoder<BytesMutWriter>;

    code_impl!(GzDecoder);
    code_impl!(GzEncoder);
}

#[cfg(feature = "de")]
mod deflate {
    use super::writer::BytesMutWriter;

    use flate2::write::{DeflateDecoder, DeflateEncoder};

    pub type Decoder = DeflateDecoder<BytesMutWriter>;
    pub type Encoder = DeflateEncoder<BytesMutWriter>;

    code_impl!(DeflateDecoder);
    code_impl!(DeflateEncoder);
}

#[cfg(feature = "zs")]
mod zstandard {
    use std::io::{self, Write};

    use bytes::Bytes;
    use zstd::stream::write::{Decoder as ZstdDecoder, Encoder as ZstdEncoder};

    use super::{coder::Code, writer::BytesMutWriter};

    pub struct Decoder(Option<ZstdDecoder<'static, BytesMutWriter>>);

    impl Decoder {
        pub(crate) fn new() -> Self {
            Self(Some(ZstdDecoder::new(BytesMutWriter::new()).unwrap()))
        }
    }

    pub struct Encoder(Option<ZstdEncoder<'static, BytesMutWriter>>);

    impl Encoder {
        pub(crate) fn new(level: i32) -> Self {
            Self(Some(ZstdEncoder::new(BytesMutWriter::new(), level).unwrap()))
        }
    }

    impl<T> Code<T> for Decoder
    where
        T: AsRef<[u8]>,
    {
        type Item = Bytes;

        fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
            let decoder = self.0.as_mut().unwrap();
            decoder.write_all(item.as_ref())?;
            decoder.flush()?;
            let b = decoder.get_mut().take();
            if !b.is_empty() { Ok(Some(b)) } else { Ok(None) }
        }

        fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
            match self.0.take() {
                Some(decoder) => {
                    let b = decoder.into_inner().take_owned();
                    Ok(Some(b))
                }
                None => Ok(None),
            }
        }

        #[inline]
        fn is_end_stream(&self, _: &impl http_body_alt::Body) -> bool {
            self.0.is_none()
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
            if !b.is_empty() { Ok(Some(b)) } else { Ok(None) }
        }

        fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
            match self.0.take() {
                Some(encoder) => {
                    let b = encoder.finish()?.take_owned();
                    Ok(Some(b))
                }
                None => Ok(None),
            }
        }

        #[inline]
        fn is_end_stream(&self, _: &impl http_body_alt::Body) -> bool {
            self.0.is_none()
        }
    }
}

pub use self::coder::{Code, Coder, FeaturedCode};
pub use self::coding::ContentEncoding;
pub use self::decode::try_decoder;
