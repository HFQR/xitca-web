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
pub fn try_decoder<S, T, E>(headers: &HeaderMap, body: S) -> Result<Coder<S, FeaturedCode>, EncodingError>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
{
    from_headers(headers).map(|decoder| Coder::new(body, decoder))
}

fn from_headers(headers: &HeaderMap) -> Result<FeaturedCode, EncodingError> {
    let mut err = None;

    for enc in headers
        .get_all(&CONTENT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match ContentEncoding::try_parse(enc) {
            Ok(encoding) => match encoding {
                ContentEncoding::NoOp => break,
                ContentEncoding::Br => {
                    #[cfg(feature = "br")]
                    return Ok(FeaturedCode::DecodeBr(super::brotli::Decoder::new(
                        super::writer::BytesMutWriter::new(),
                    )));
                }
                ContentEncoding::Gzip => {
                    #[cfg(feature = "gz")]
                    return Ok(FeaturedCode::DecodeGz(super::gzip::Decoder::new(
                        super::writer::BytesMutWriter::new(),
                    )));
                }
                ContentEncoding::Deflate => {
                    #[cfg(feature = "de")]
                    return Ok(FeaturedCode::DecodeDe(super::deflate::Decoder::new(
                        super::writer::BytesMutWriter::new(),
                    )));
                }
            },
            Err(e) => err = Some(e),
        };
    }

    if let Some(e) = err {
        Err(e.into())
    } else {
        Ok(FeaturedCode::default())
    }
}
