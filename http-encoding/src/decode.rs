//! Stream decoders.

use futures_core::Stream;
use http::header::{HeaderMap, CONTENT_ENCODING};

use super::{
    coder::{Coder, FeaturedCode},
    coding::ContentEncoding,
    error::EncodingError,
};

/// Construct from headers and stream body. Use for decoding.
pub fn try_decoder<Req, S, T, E>(req: Req, body: S) -> Result<Coder<S, FeaturedCode>, EncodingError>
where
    Req: std::borrow::Borrow<http::Request<()>>,
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
{
    let decoder = from_headers(req.borrow().headers())?;
    Ok(Coder::new(body, decoder))
}

fn from_headers(headers: &HeaderMap) -> Result<FeaturedCode, EncodingError> {
    match headers.get(&CONTENT_ENCODING) {
        None => Ok(FeaturedCode::default()),
        Some(value) => {
            let encoding = value.to_str().map_err(|_| EncodingError::ParseAcceptEncoding)?;
            match ContentEncoding::try_parse(encoding)? {
                ContentEncoding::Br => {
                    #[cfg(feature = "br")]
                    {
                        Ok(FeaturedCode::DecodeBr(super::brotli::Decoder::new(
                            super::writer::Writer::new(),
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
                            super::writer::Writer::new(),
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
                            super::writer::Writer::new(),
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
    }
}
