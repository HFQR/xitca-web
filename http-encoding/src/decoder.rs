//! Stream decoders.

use std::{future::Future, io};

use bytes::Bytes;
use futures_core::Stream;
use http::header::{HeaderMap, CONTENT_ENCODING};

#[cfg(feature = "br")]
use super::brotli::BrotliDecoder;
#[cfg(feature = "de")]
use super::deflate::DeflateDecoder;
#[cfg(feature = "gz")]
use super::gzip::GzDecoder;
#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
use super::writer::Writer;

use super::coder::{AsyncCode, Coder, CoderError, IdentityCoder};
use crate::coding::ContentEncoding;

impl<S, De, T, E> Coder<S, De, T>
where
    S: Stream<Item = Result<T, E>>,
    De: AsyncCode<T>,
    T: AsRef<[u8]> + Send + 'static,
    Bytes: From<T>,
{
    /// Construct from headers and stream body. Use for decoding.
    pub fn try_decoder_from_parts(headers: &HeaderMap, body: S) -> Result<Coder<S, ContentDecoder, T>, CoderError<E>> {
        let decoder = from_headers(headers)?;
        Ok(Coder::new(body, decoder))
    }
}

fn from_headers<E>(headers: &HeaderMap) -> Result<ContentDecoder, CoderError<E>> {
    let decoder = headers
        .get(&CONTENT_ENCODING)
        .and_then(|val| val.to_str().ok())
        .map(|encoding| match ContentEncoding::try_from(encoding)? {
            ContentEncoding::Br => {
                #[cfg(feature = "br")]
                {
                    Ok(_ContentDecoder::Br(super::brotli::BrotliDecoder::new(Writer::new())))
                }
                #[cfg(not(feature = "br"))]
                Err(CoderError::Feature(super::coder::Feature::Br))
            }
            ContentEncoding::Gzip => {
                #[cfg(feature = "gz")]
                {
                    Ok(_ContentDecoder::Gz(super::gzip::GzDecoder::new(Writer::new())))
                }
                #[cfg(not(feature = "gz"))]
                Err(CoderError::Feature(super::coder::Feature::Gzip))
            }
            ContentEncoding::Deflate => {
                #[cfg(feature = "de")]
                {
                    Ok(_ContentDecoder::De(super::deflate::DeflateDecoder::new(Writer::new())))
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

impl<Item> AsyncCode<Item> for ContentDecoder
where
    Item: AsRef<[u8]> + Send + 'static,
    Bytes: From<Item>,
{
    type Item = Bytes;
    type Future = impl Future<Output = io::Result<(Self, Option<Self::Item>)>>;

    fn code(self, item: Item) -> io::Result<(Self, Option<Self::Item>)> {
        match self.decoder {
            _ContentDecoder::Identity(decoder) => {
                <IdentityCoder as AsyncCode<Item>>::code(decoder, item).map(|(decoder, item)| (decoder.into(), item))
            }
            #[cfg(feature = "br")]
            _ContentDecoder::Br(decoder) => <BrotliDecoder<Writer> as AsyncCode<Item>>::code(decoder, item)
                .map(|(decoder, item)| (decoder.into(), item)),
            #[cfg(feature = "gz")]
            _ContentDecoder::Gz(decoder) => <GzDecoder<Writer> as AsyncCode<Item>>::code(decoder, item)
                .map(|(decoder, item)| (decoder.into(), item)),
            #[cfg(feature = "de")]
            _ContentDecoder::De(decoder) => <DeflateDecoder<Writer> as AsyncCode<Item>>::code(decoder, item)
                .map(|(decoder, item)| (decoder.into(), item)),
        }
    }

    fn code_async(self, item: Item) -> Self::Future {
        async move {
            match self.decoder {
                _ContentDecoder::Identity(decoder) => <IdentityCoder as AsyncCode<Item>>::code_async(decoder, item)
                    .await
                    .map(|(decoder, item)| (decoder.into(), item)),
                #[cfg(feature = "br")]
                _ContentDecoder::Br(decoder) => <BrotliDecoder<Writer> as AsyncCode<Item>>::code_async(decoder, item)
                    .await
                    .map(|(decoder, item)| (decoder.into(), item)),
                #[cfg(feature = "gz")]
                _ContentDecoder::Gz(decoder) => <GzDecoder<Writer> as AsyncCode<Item>>::code_async(decoder, item)
                    .await
                    .map(|(decoder, item)| (decoder.into(), item)),
                #[cfg(feature = "de")]
                _ContentDecoder::De(decoder) => <DeflateDecoder<Writer> as AsyncCode<Item>>::code_async(decoder, item)
                    .await
                    .map(|(decoder, item)| (decoder.into(), item)),
            }
        }
    }

    fn code_eof(self) -> io::Result<Option<Self::Item>> {
        match self.decoder {
            _ContentDecoder::Identity(decoder) => <IdentityCoder as AsyncCode<Item>>::code_eof(decoder),
            #[cfg(feature = "br")]
            _ContentDecoder::Br(decoder) => <BrotliDecoder<Writer> as AsyncCode<Item>>::code_eof(decoder),
            #[cfg(feature = "gz")]
            _ContentDecoder::Gz(decoder) => <GzDecoder<Writer> as AsyncCode<Item>>::code_eof(decoder),
            #[cfg(feature = "de")]
            _ContentDecoder::De(decoder) => <DeflateDecoder<Writer> as AsyncCode<Item>>::code_eof(decoder),
        }
    }
}
