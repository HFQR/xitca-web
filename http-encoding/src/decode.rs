//! Stream decoders.

use futures_core::Stream;
use http::header::{HeaderMap, CONTENT_ENCODING};

use super::{
    coder::{Coder, CoderError, FeaturedCode, NoOpCode},
    coding::ContentEncoding,
};

/// Construct from headers and stream body. Use for decoding.
pub fn try_decoder<Req, S, T, E>(req: Req, body: S) -> Result<Coder<S, FeaturedCode>, CoderError<E>>
where
    Req: std::borrow::Borrow<http::Request<()>>,
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
{
    let decoder = from_headers(req.borrow().headers())?;
    Ok(Coder::new(body, decoder))
}

fn from_headers<E>(headers: &HeaderMap) -> Result<FeaturedCode, CoderError<E>> {
    let decoder = headers
        .get(&CONTENT_ENCODING)
        .and_then(|val| val.to_str().ok())
        .map(|encoding| match ContentEncoding::from(encoding) {
            ContentEncoding::Br => {
                #[cfg(feature = "br")]
                {
                    Ok(FeaturedCode::DecodeBr(super::brotli::Decoder::new(
                        super::writer::Writer::new(),
                    )))
                }
                #[cfg(not(feature = "br"))]
                Err(CoderError::Feature(super::coder::Feature::Br))
            }
            ContentEncoding::Gzip => {
                #[cfg(feature = "gz")]
                {
                    Ok(FeaturedCode::DecodeGz(super::gzip::Decoder::new(
                        super::writer::Writer::new(),
                    )))
                }
                #[cfg(not(feature = "gz"))]
                Err(CoderError::Feature(super::coder::Feature::Gzip))
            }
            ContentEncoding::Deflate => {
                #[cfg(feature = "de")]
                {
                    Ok(FeaturedCode::DecodeDe(super::deflate::Decoder::new(
                        super::writer::Writer::new(),
                    )))
                }
                #[cfg(not(feature = "de"))]
                Err(CoderError::Feature(super::coder::Feature::Deflate))
            }
            ContentEncoding::NoOp => Ok(FeaturedCode::NoOp(NoOpCode)),
        })
        .unwrap_or_else(|| Ok::<_, CoderError<E>>(FeaturedCode::NoOp(NoOpCode)))?;

    Ok(decoder)
}
