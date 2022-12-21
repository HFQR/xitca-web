//! Stream decoders.

use futures_core::Stream;
use http::header::{HeaderMap, CONTENT_ENCODING};

use super::{
    coder::{Coder, FeaturedCode},
    coding::ContentEncoding,
    error::EncodingError,
};

/// Construct from headers and stream body. Use for decoding.
#[inline]
pub fn try_decoder<Req, S, T, E>(req: Req, body: S) -> Result<Coder<S, FeaturedCode>, EncodingError>
where
    Req: std::borrow::Borrow<http::Request<()>>,
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
{
    from_headers(req.borrow().headers()).map(|decoder| Coder::new(body, decoder))
}

fn from_headers(headers: &HeaderMap) -> Result<FeaturedCode, EncodingError> {
    let Some(val) = headers.get(&CONTENT_ENCODING) else { return Ok(FeaturedCode::default()) };
    let enc = val.to_str().map_err(|_| EncodingError::ParseAcceptEncoding)?;
    match ContentEncoding::try_parse(enc)? {
        ContentEncoding::Br => {
            #[cfg(feature = "br")]
            {
                Ok(FeaturedCode::DecodeBr(super::brotli::Decoder::new(
                    super::writer::BytesMutWriter::new(),
                )))
            }
            #[cfg(not(feature = "br"))]
            {
                Err(super::error::FeatureError::Br.into())
            }
        }
        ContentEncoding::Gzip => {
            #[cfg(feature = "gz")]
            {
                Ok(FeaturedCode::DecodeGz(super::gzip::Decoder::new(
                    super::writer::BytesMutWriter::new(),
                )))
            }
            #[cfg(not(feature = "gz"))]
            {
                Err(super::error::FeatureError::Gzip.into())
            }
        }
        ContentEncoding::Deflate => {
            #[cfg(feature = "de")]
            {
                Ok(FeaturedCode::DecodeDe(super::deflate::Decoder::new(
                    super::writer::BytesMutWriter::new(),
                )))
            }
            #[cfg(not(feature = "de"))]
            {
                Err(super::error::FeatureError::Deflate.into())
            }
        }
        ContentEncoding::NoOp => Ok(FeaturedCode::default()),
    }
}
