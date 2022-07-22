//! Stream encoders.

use std::io;

use bytes::Bytes;
use futures_core::Stream;
use http::{header, Response, StatusCode};

#[cfg(feature = "br")]
use super::brotli::BrotliEncoder;
#[cfg(feature = "de")]
use super::deflate::DeflateEncoder;
#[cfg(feature = "gz")]
use super::gzip::GzEncoder;
#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
use super::writer::Writer;

use super::coder::{Code, Coder, IdentityCoder};
use super::coding::ContentEncoding;
use crate::coder::CoderError;

/// Construct from headers and stream body. Use for encoding.
pub fn try_encoder<S, T, E>(
    response: Response<S>,
    encoding: ContentEncoding,
) -> Result<Response<Coder<S, ContentEncoder>>, CoderError<E>>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + Send + 'static,
{
    #[allow(unused_mut)]
    let (mut parts, body) = response.into_parts();

    let can_encode = !(parts.headers.contains_key(&header::CONTENT_ENCODING)
        || parts.status == StatusCode::SWITCHING_PROTOCOLS
        || parts.status == StatusCode::NO_CONTENT
        || encoding == ContentEncoding::Identity
        || encoding == ContentEncoding::Auto);

    let encoder = {
        if can_encode {
            match encoding {
                ContentEncoding::Deflate => {
                    #[cfg(feature = "de")]
                    {
                        update_header(&mut parts.headers, "deflate");
                        _ContentEncoder::De(DeflateEncoder::new(Writer::new(), flate2::Compression::fast()))
                    }
                    #[cfg(not(feature = "de"))]
                    return Err(CoderError::Feature(super::coder::Feature::Deflate));
                }
                ContentEncoding::Gzip => {
                    #[cfg(feature = "gz")]
                    {
                        update_header(&mut parts.headers, "gzip");
                        _ContentEncoder::Gz(GzEncoder::new(Writer::new(), flate2::Compression::fast()))
                    }
                    #[cfg(not(feature = "gz"))]
                    return Err(CoderError::Feature(super::coder::Feature::Gzip));
                }
                ContentEncoding::Br => {
                    #[cfg(feature = "br")]
                    {
                        update_header(&mut parts.headers, "br");
                        _ContentEncoder::Br(BrotliEncoder::new(Writer::new(), 3))
                    }
                    #[cfg(not(feature = "br"))]
                    return Err(CoderError::Feature(super::coder::Feature::Br));
                }
                ContentEncoding::Identity | ContentEncoding::Auto => {
                    // Since identity does not do actual encoding.
                    // content-length header can left untouched.
                    parts
                        .headers
                        .insert(header::CONTENT_ENCODING, header::HeaderValue::from_static("identity"));

                    _ContentEncoder::Identity(IdentityCoder)
                }
            }
        } else {
            // this branch serves solely as pass through so headers can be left untouched.
            _ContentEncoder::Identity(IdentityCoder)
        }
    };

    let encoder = ContentEncoder { encoder };

    let body = Coder::new(body, encoder);

    let response = Response::from_parts(parts, body);

    Ok(response)
}

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
fn update_header(headers: &mut header::HeaderMap, value: &'static str) {
    headers.insert(header::CONTENT_ENCODING, header::HeaderValue::from_static(value));
    headers.remove(header::CONTENT_LENGTH);
    headers.insert(header::TRANSFER_ENCODING, header::HeaderValue::from_static("chunked"));
}

pub struct ContentEncoder {
    encoder: _ContentEncoder,
}

enum _ContentEncoder {
    Identity(IdentityCoder),
    #[cfg(feature = "br")]
    Br(BrotliEncoder<Writer>),
    #[cfg(feature = "gz")]
    Gz(GzEncoder<Writer>),
    #[cfg(feature = "de")]
    De(DeflateEncoder<Writer>),
}

impl From<IdentityCoder> for ContentEncoder {
    fn from(encoder: IdentityCoder) -> Self {
        Self {
            encoder: _ContentEncoder::Identity(encoder),
        }
    }
}

#[cfg(feature = "br")]
impl From<BrotliEncoder<Writer>> for ContentEncoder {
    fn from(encoder: BrotliEncoder<Writer>) -> Self {
        Self {
            encoder: _ContentEncoder::Br(encoder),
        }
    }
}

#[cfg(feature = "gz")]
impl From<GzEncoder<Writer>> for ContentEncoder {
    fn from(encoder: GzEncoder<Writer>) -> Self {
        Self {
            encoder: _ContentEncoder::Gz(encoder),
        }
    }
}

#[cfg(feature = "de")]
impl From<DeflateEncoder<Writer>> for ContentEncoder {
    fn from(encoder: DeflateEncoder<Writer>) -> Self {
        Self {
            encoder: _ContentEncoder::De(encoder),
        }
    }
}

impl<T> Code<T> for ContentEncoder
where
    T: AsRef<[u8]> + Send + 'static,
{
    type Item = Bytes;

    fn code(&mut self, item: T) -> io::Result<Option<Self::Item>> {
        match self.encoder {
            _ContentEncoder::Identity(ref mut encoder) => <IdentityCoder as Code<T>>::code(encoder, item),
            #[cfg(feature = "br")]
            _ContentEncoder::Br(ref mut encoder) => <BrotliEncoder<Writer> as Code<T>>::code(encoder, item),
            #[cfg(feature = "gz")]
            _ContentEncoder::Gz(ref mut encoder) => <GzEncoder<Writer> as Code<T>>::code(encoder, item),
            #[cfg(feature = "de")]
            _ContentEncoder::De(ref mut encoder) => <DeflateEncoder<Writer> as Code<T>>::code(encoder, item),
        }
    }

    fn code_eof(&mut self) -> io::Result<Option<Self::Item>> {
        match self.encoder {
            _ContentEncoder::Identity(ref mut encoder) => <IdentityCoder as Code<T>>::code_eof(encoder),
            #[cfg(feature = "br")]
            _ContentEncoder::Br(ref mut encoder) => <BrotliEncoder<Writer> as Code<T>>::code_eof(encoder),
            #[cfg(feature = "gz")]
            _ContentEncoder::Gz(ref mut encoder) => <GzEncoder<Writer> as Code<T>>::code_eof(encoder),
            #[cfg(feature = "de")]
            _ContentEncoder::De(ref mut encoder) => <DeflateEncoder<Writer> as Code<T>>::code_eof(encoder),
        }
    }
}
