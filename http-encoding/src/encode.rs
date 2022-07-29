//! Stream encoders.

use futures_core::Stream;
use http::{header, Response, StatusCode};

use super::{
    coder::{Coder, FeaturedCode, NoOpCode},
    coding::ContentEncoding,
};

/// Construct from headers and stream body. Use for encoding.
pub fn encoder<S, T, E>(response: Response<S>, encoding: ContentEncoding) -> Response<Coder<S, FeaturedCode>>
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
        match encoding {
            #[cfg(feature = "de")]
            ContentEncoding::Deflate if can_encode => {
                update_header(&mut parts.headers, "deflate");
                FeaturedCode::EncodeDe(super::deflate::Encoder::new(
                    super::writer::Writer::new(),
                    flate2::Compression::fast(),
                ))
            }
            #[cfg(feature = "gz")]
            ContentEncoding::Gzip if can_encode => {
                update_header(&mut parts.headers, "gzip");
                FeaturedCode::EncodeGz(super::gzip::Encoder::new(
                    super::writer::Writer::new(),
                    flate2::Compression::fast(),
                ))
            }
            #[cfg(feature = "br")]
            ContentEncoding::Br if can_encode => {
                update_header(&mut parts.headers, "br");
                FeaturedCode::EncodeBr(super::brotli::Encoder::new(3))
            }
            _ => FeaturedCode::NoOp(NoOpCode),
        }
    };

    let body = Coder::new(body, encoder);
    Response::from_parts(parts, body)
}

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
fn update_header(headers: &mut header::HeaderMap, value: &'static str) {
    headers.insert(header::CONTENT_ENCODING, header::HeaderValue::from_static(value));
    headers.remove(header::CONTENT_LENGTH);
    headers.insert(header::TRANSFER_ENCODING, header::HeaderValue::from_static("chunked"));
}
