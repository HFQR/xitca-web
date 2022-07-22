//! Stream decoders.

use std::io;

use bytes::Bytes;
use futures_core::Stream;
use http::header::{HeaderMap, CONTENT_ENCODING};

use crate::coding::ContentEncoding;

#[cfg(feature = "br")]
use super::brotli::BrotliDecoder;
#[cfg(feature = "de")]
use super::deflate::DeflateDecoder;
#[cfg(feature = "gz")]
use super::gzip::GzDecoder;
#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
use super::writer::Writer;

use super::coder::{Code, Coder, CoderError, IdentityCoder};

/// Construct from headers and stream body. Use for decoding.
pub fn try_decoder<Req, S, T, E>(req: Req, body: S) -> Result<Coder<S, ContentDecoder>, CoderError<E>>
where
    Req: std::borrow::Borrow<http::Request<()>>,
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
{
    let decoder = from_headers(req.borrow().headers())?;
    Ok(Coder::new(body, decoder))
}

fn from_headers<E>(headers: &HeaderMap) -> Result<ContentDecoder, CoderError<E>> {
    let decoder = headers
        .get(&CONTENT_ENCODING)
        .and_then(|val| val.to_str().ok())
        .map(|encoding| match ContentEncoding::from(encoding) {
            ContentEncoding::Br => {
                #[cfg(feature = "br")]
                {
                    Ok(_ContentDecoder::Br(BrotliDecoder::new(Writer::new())))
                }
                #[cfg(not(feature = "br"))]
                Err(CoderError::Feature(super::coder::Feature::Br))
            }
            ContentEncoding::Gzip => {
                #[cfg(feature = "gz")]
                {
                    Ok(_ContentDecoder::Gz(GzDecoder::new(Writer::new())))
                }
                #[cfg(not(feature = "gz"))]
                Err(CoderError::Feature(super::coder::Feature::Gzip))
            }
            ContentEncoding::Deflate => {
                #[cfg(feature = "de")]
                {
                    Ok(_ContentDecoder::De(DeflateDecoder::new(Writer::new())))
                }
                #[cfg(not(feature = "de"))]
                Err(CoderError::Feature(super::coder::Feature::Deflate))
            }
            ContentEncoding::Identity | ContentEncoding::Auto => Ok(_ContentDecoder::Identity(IdentityCoder)),
        })
        .unwrap_or_else(|| Ok::<_, CoderError<E>>(_ContentDecoder::Identity(IdentityCoder)))?;

    Ok(ContentDecoder { decoder })
}

pub struct ContentDecoder {
    decoder: _ContentDecoder,
}

enum _ContentDecoder {
    Identity(IdentityCoder),
    #[cfg(feature = "br")]
    Br(BrotliDecoder<Writer>),
    #[cfg(feature = "gz")]
    Gz(GzDecoder<Writer>),
    #[cfg(feature = "de")]
    De(DeflateDecoder<Writer>),
}

impl From<IdentityCoder> for ContentDecoder {
    fn from(decoder: IdentityCoder) -> Self {
        Self {
            decoder: _ContentDecoder::Identity(decoder),
        }
    }
}

#[cfg(feature = "br")]
impl From<BrotliDecoder<Writer>> for ContentDecoder {
    fn from(decoder: BrotliDecoder<Writer>) -> Self {
        Self {
            decoder: _ContentDecoder::Br(decoder),
        }
    }
}

#[cfg(feature = "gz")]
impl From<GzDecoder<Writer>> for ContentDecoder {
    fn from(decoder: GzDecoder<Writer>) -> Self {
        Self {
            decoder: _ContentDecoder::Gz(decoder),
        }
    }
}

#[cfg(feature = "de")]
impl From<DeflateDecoder<Writer>> for ContentDecoder {
    fn from(decoder: DeflateDecoder<Writer>) -> Self {
        Self {
            decoder: _ContentDecoder::De(decoder),
        }
    }
}

impl<T> Code<T> for ContentDecoder
where
    T: AsRef<[u8]> + 'static,
{
    type Item = Bytes;

    fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
        match self.decoder {
            _ContentDecoder::Identity(ref mut decoder) => IdentityCoder::code(decoder, item),
            #[cfg(feature = "br")]
            _ContentDecoder::Br(ref mut decoder) => BrotliDecoder::<Writer>::code(decoder, item),
            #[cfg(feature = "gz")]
            _ContentDecoder::Gz(ref mut decoder) => GzDecoder::<Writer>::code(decoder, item),
            #[cfg(feature = "de")]
            _ContentDecoder::De(ref mut decoder) => DeflateDecoder::<Writer>::code(decoder, item),
        }
    }

    fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
        match self.decoder {
            _ContentDecoder::Identity(ref mut decoder) => <IdentityCoder as Code<T>>::code_eof(decoder),
            #[cfg(feature = "br")]
            _ContentDecoder::Br(ref mut decoder) => <BrotliDecoder<Writer> as Code<T>>::code_eof(decoder),
            #[cfg(feature = "gz")]
            _ContentDecoder::Gz(ref mut decoder) => <GzDecoder<Writer> as Code<T>>::code_eof(decoder),
            #[cfg(feature = "de")]
            _ContentDecoder::De(ref mut decoder) => <DeflateDecoder<Writer> as Code<T>>::code_eof(decoder),
        }
    }
}
