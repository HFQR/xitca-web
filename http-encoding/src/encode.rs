//! Stream encoders.

use futures_core::Stream;
use http::{header, Response, StatusCode};

use super::{
    coder::{Coder, FeaturedCode},
    coding::ContentEncoding,
};

/// Construct from headers and stream body. Use for encoding.
pub fn encoder<S, T, E>(response: Response<S>, mut encoding: ContentEncoding) -> Response<Coder<S, FeaturedCode>>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
{
    #[allow(unused_mut)]
    let (mut parts, body) = response.into_parts();

    if parts.headers.contains_key(&header::CONTENT_ENCODING)
        || parts.status == StatusCode::SWITCHING_PROTOCOLS
        || parts.status == StatusCode::NO_CONTENT
    {
        encoding = ContentEncoding::NoOp
    }

    let encoder = {
        match encoding {
            #[cfg(feature = "de")]
            ContentEncoding::Deflate => {
                update_header(&mut parts.headers, "deflate", parts.version);
                FeaturedCode::EncodeDe(super::deflate::Encoder::new(
                    super::writer::BytesMutWriter::new(),
                    flate2::Compression::fast(),
                ))
            }
            #[cfg(feature = "gz")]
            ContentEncoding::Gzip => {
                update_header(&mut parts.headers, "gzip", parts.version);
                FeaturedCode::EncodeGz(super::gzip::Encoder::new(
                    super::writer::BytesMutWriter::new(),
                    flate2::Compression::fast(),
                ))
            }
            #[cfg(feature = "br")]
            ContentEncoding::Br => {
                update_header(&mut parts.headers, "br", parts.version);
                FeaturedCode::EncodeBr(super::brotli::Encoder::new(3))
            }
            _ => FeaturedCode::default(),
        }
    };

    let body = Coder::new(body, encoder);
    Response::from_parts(parts, body)
}

#[cfg(any(feature = "br", feature = "gz", feature = "de"))]
fn update_header(headers: &mut header::HeaderMap, value: &'static str, version: http::Version) {
    headers.insert(header::CONTENT_ENCODING, header::HeaderValue::from_static(value));
    headers.remove(header::CONTENT_LENGTH);

    // Connection specific headers are not allowed in HTTP/2 and later versions.
    // see https://datatracker.ietf.org/doc/html/rfc7540#section-8.1.2.2
    if version < http::Version::HTTP_2 {
        headers.insert(header::TRANSFER_ENCODING, header::HeaderValue::from_static("chunked"));
    }
}
