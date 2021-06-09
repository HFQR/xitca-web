//! Stream encoders.

use std::{future::Future, io};

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

use super::coder::{AsyncCode, Coder, IdentityCoder};
use super::coding::ContentEncoding;
use crate::coder::CoderError;

type Res<S, De, Fut> = Response<Coder<S, De, Fut>>;

impl<S, De, T, E> Coder<S, De, De::Future>
where
    S: Stream<Item = Result<T, E>>,
    De: AsyncCode<T>,
    T: AsRef<[u8]> + Send + 'static,
    Bytes: From<T>,
{
    /// Construct from headers and stream body. Use for encoding.
    pub fn try_encoder_from_response(
        response: Response<S>,
        encoding: ContentEncoding,
    ) -> Result<Res<S, ContentEncoder, <ContentEncoder as AsyncCode<T>>::Future>, CoderError<E>> {
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
                            _ContentEncoder::De(super::deflate::DeflateEncoder::new(
                                Writer::new(),
                                flate2::Compression::fast(),
                            ))
                        }
                        #[cfg(not(feature = "de"))]
                        return Err(CoderError::Feature(super::coder::Feature::Deflate));
                    }
                    ContentEncoding::Gzip => {
                        #[cfg(feature = "gz")]
                        {
                            update_header(&mut parts.headers, "gzip");
                            _ContentEncoder::Gz(super::gzip::GzEncoder::new(Writer::new(), flate2::Compression::fast()))
                        }
                        #[cfg(not(feature = "gz"))]
                        return Err(CoderError::Feature(super::coder::Feature::Gzip));
                    }
                    ContentEncoding::Br => {
                        #[cfg(feature = "br")]
                        {
                            update_header(&mut parts.headers, "br");
                            _ContentEncoder::Br(super::brotli::BrotliEncoder::new(Writer::new(), 3))
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

impl<Item> AsyncCode<Item> for ContentEncoder
where
    Item: AsRef<[u8]> + Send + 'static,
    Bytes: From<Item>,
{
    type Item = Bytes;
    type Future = impl Future<Output = io::Result<(Self, Option<Self::Item>)>>;

    fn code(self, item: Item) -> Self::Future {
        async move {
            match self.encoder {
                _ContentEncoder::Identity(encoder) => <IdentityCoder as AsyncCode<Item>>::code(encoder, item)
                    .await
                    .map(|(encoder, item)| (encoder.into(), item)),
                #[cfg(feature = "br")]
                _ContentEncoder::Br(encoder) => <BrotliEncoder<Writer> as AsyncCode<Item>>::code(encoder, item)
                    .await
                    .map(|(encoder, item)| (encoder.into(), item)),
                #[cfg(feature = "gz")]
                _ContentEncoder::Gz(encoder) => <GzEncoder<Writer> as AsyncCode<Item>>::code(encoder, item)
                    .await
                    .map(|(encoder, item)| (encoder.into(), item)),
                #[cfg(feature = "de")]
                _ContentEncoder::De(encoder) => <DeflateEncoder<Writer> as AsyncCode<Item>>::code(encoder, item)
                    .await
                    .map(|(encoder, item)| (encoder.into(), item)),
            }
        }
    }

    fn code_eof(self) -> io::Result<Option<Self::Item>> {
        match self.encoder {
            _ContentEncoder::Identity(encoder) => <IdentityCoder as AsyncCode<Item>>::code_eof(encoder),
            #[cfg(feature = "br")]
            _ContentEncoder::Br(encoder) => <BrotliEncoder<Writer> as AsyncCode<Item>>::code_eof(encoder),
            #[cfg(feature = "gz")]
            _ContentEncoder::Gz(encoder) => <GzEncoder<Writer> as AsyncCode<Item>>::code_eof(encoder),
            #[cfg(feature = "de")]
            _ContentEncoder::De(encoder) => <DeflateEncoder<Writer> as AsyncCode<Item>>::code_eof(encoder),
        }
    }
}
