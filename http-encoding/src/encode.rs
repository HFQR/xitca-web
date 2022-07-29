//! Stream encoders.

use futures_core::Stream;
use http::{header, Response, StatusCode};

use super::{
    coder::{Coder, CoderError, FeaturedCode, NoOpCode},
    coding::ContentEncoding,
};

/// Construct from headers and stream body. Use for encoding.
pub fn try_encoder<S, T, E>(
    response: Response<S>,
    encoding: ContentEncoding,
) -> Result<Response<Coder<S, FeaturedCode>>, CoderError<E>>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
{
    #[allow(unused_mut)]
    let (mut parts, body) = response.into_parts();

    let can_encode = !(parts.headers.contains_key(&header::CONTENT_ENCODING)
        || parts.status == StatusCode::SWITCHING_PROTOCOLS
        || parts.status == StatusCode::NO_CONTENT);

    let encoder = {
        if can_encode {
            match encoding {
                ContentEncoding::Deflate => {
                    #[cfg(feature = "de")]
                    {
                        update_header(&mut parts.headers, "deflate");
                        FeaturedCode::EncodeDe(super::deflate::Encoder::new(
                            super::writer::Writer::new(),
                            flate2::Compression::fast(),
                        ))
                    }
                    #[cfg(not(feature = "de"))]
                    return Err(CoderError::Feature(super::coder::FeatureError::Deflate));
                }
                ContentEncoding::Gzip => {
                    #[cfg(feature = "gz")]
                    {
                        update_header(&mut parts.headers, "gzip");
                        FeaturedCode::EncodeGz(super::gzip::Encoder::new(
                            super::writer::Writer::new(),
                            flate2::Compression::fast(),
                        ))
                    }
                    #[cfg(not(feature = "gz"))]
                    return Err(CoderError::Feature(super::coder::FeatureError::Gzip));
                }
                ContentEncoding::Br => {
                    #[cfg(feature = "br")]
                    {
                        update_header(&mut parts.headers, "br");
                        FeaturedCode::EncodeBr(super::brotli::Encoder::new(super::writer::Writer::new(), 3))
                    }
                    #[cfg(not(feature = "br"))]
                    return Err(CoderError::Feature(super::coder::FeatureError::Br));
                }
                ContentEncoding::NoOp => {
                    // Since identity does not do actual encoding. content-length header can left untouched.
                    FeaturedCode::NoOp(NoOpCode)
                }
            }
        } else {
            // this branch serves solely as pass through so headers can be left untouched.
            FeaturedCode::NoOp(NoOpCode)
        }
    };

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
