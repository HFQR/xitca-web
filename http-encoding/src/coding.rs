use http::{
    HeaderValue, Response, StatusCode,
    header::{self, ACCEPT_ENCODING, HeaderMap, HeaderName},
};
use http_body_alt::Body;

use super::{
    coder::{Coder, FeaturedCode},
    error::FeatureError,
};

/// Represents a supported content encoding.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ContentEncoding {
    /// A format using the Brotli algorithm.
    Br,
    /// A format using the zlib structure with deflate algorithm.
    Deflate,
    /// Gzip algorithm.
    Gzip,
    /// Zstandard algorithm.
    Zstd,
    /// Indicates no operation is done with encoding.
    #[default]
    Identity,
}

impl ContentEncoding {
    pub const fn as_header_value(&self) -> HeaderValue {
        match self {
            Self::Br => HeaderValue::from_static("br"),
            Self::Deflate => HeaderValue::from_static("deflate"),
            Self::Gzip => HeaderValue::from_static("gzip"),
            Self::Zstd => HeaderValue::from_static("zstd"),
            Self::Identity => HeaderValue::from_static("identity"),
        }
    }

    /// Negotiate content encoding from the `accept-encoding` header.
    ///
    /// Returns the highest q-value encoding that is supported by enabled crate features.
    /// Returns [`ContentEncoding::NoOp`] if no supported encoding is found.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut prefer = ContentEncodingWithQValue::default();

        for encoding in Self::_from_headers(headers, &ACCEPT_ENCODING) {
            prefer.try_update(encoding);
        }

        prefer.enc
    }

    /// Negotiate content encoding from a caller-specified header name.
    ///
    /// Same as [`from_headers`](Self::from_headers) but reads from `needle` instead of
    /// `accept-encoding`. Useful for protocols that use a different header for encoding
    /// negotiation (e.g. `grpc-accept-encoding` for gRPC).
    pub fn from_headers_with(headers: &HeaderMap, needle: &HeaderName) -> Self {
        let mut prefer = ContentEncodingWithQValue::default();

        for encoding in Self::_from_headers(headers, needle) {
            prefer.try_update(encoding);
        }

        prefer.enc
    }

    /// Encode a response body, updating response headers accordingly.
    ///
    /// Skips encoding if the response already has a `Content-Encoding` header, or if the
    /// status is `101 Switching Protocols` or `204 No Content`.
    ///
    /// # Headers
    /// Sets `Content-Encoding` and removes `Content-Length` when compression is applied.
    /// For HTTP/1.1 responses, `Transfer-Encoding: chunked` is appended. Callers should avoid
    /// modifying `Content-Encoding` or `Transfer-Encoding` headers after calling this method,
    /// as inconsistent values may cause incorrect behavior in downstream clients.
    pub fn try_encode<S>(mut self, response: Response<S>) -> Response<Coder<S, FeaturedCode>>
    where
        S: Body,
        S::Data: AsRef<[u8]> + 'static,
    {
        #[allow(unused_mut)]
        let (mut parts, body) = response.into_parts();

        if parts.headers.contains_key(&header::CONTENT_ENCODING)
            || parts.status == StatusCode::SWITCHING_PROTOCOLS
            || parts.status == StatusCode::NO_CONTENT
        {
            self = ContentEncoding::Identity;
        }

        self.update_header(&mut parts.headers, parts.version);

        let body = self.encode_body(body);
        Response::from_parts(parts, body)
    }

    /// Encode a [`Body`] with featured encoder
    pub fn encode_body<S>(self, body: S) -> Coder<S, FeaturedCode>
    where
        S: Body,
        S::Data: AsRef<[u8]> + 'static,
    {
        let encoder = match self {
            #[cfg(feature = "de")]
            ContentEncoding::Deflate => FeaturedCode::EncodeDe(super::deflate::Encoder::new(
                super::writer::BytesMutWriter::new(),
                flate2::Compression::fast(),
            )),
            #[cfg(feature = "gz")]
            ContentEncoding::Gzip => FeaturedCode::EncodeGz(super::gzip::Encoder::new(
                super::writer::BytesMutWriter::new(),
                flate2::Compression::fast(),
            )),
            #[cfg(feature = "br")]
            ContentEncoding::Br => FeaturedCode::EncodeBr(super::brotli::Encoder::new(3)),
            #[cfg(feature = "zs")]
            ContentEncoding::Zstd => FeaturedCode::EncodeZs(super::zstandard::Encoder::new(3)),
            _ => FeaturedCode::default(),
        };

        Coder::new(body, encoder)
    }

    /// Decode a [`Body`] with featured decoder.
    ///
    /// Symmetric to [`encode_body`](Self::encode_body). Use this when decoding outside of
    /// an HTTP response context (e.g. gRPC request decompression) where the encoding is
    /// determined by a protocol-specific header rather than `Content-Encoding`.
    pub fn decode_body<S>(self, body: S) -> Coder<S, FeaturedCode>
    where
        S: Body,
        S::Data: AsRef<[u8]> + 'static,
    {
        let decoder = match self {
            #[cfg(feature = "de")]
            ContentEncoding::Deflate => {
                FeaturedCode::DecodeDe(super::deflate::Decoder::new(super::writer::BytesMutWriter::new()))
            }
            #[cfg(feature = "gz")]
            ContentEncoding::Gzip => {
                FeaturedCode::DecodeGz(super::gzip::Decoder::new(super::writer::BytesMutWriter::new()))
            }
            #[cfg(feature = "br")]
            ContentEncoding::Br => FeaturedCode::DecodeBr(super::brotli::Decoder::new()),
            #[cfg(feature = "zs")]
            ContentEncoding::Zstd => FeaturedCode::DecodeZs(super::zstandard::Decoder::new()),
            _ => FeaturedCode::default(),
        };

        Coder::new(body, decoder)
    }

    fn update_header(self, headers: &mut HeaderMap, version: http::Version) {
        if matches!(self, ContentEncoding::Identity) {
            return;
        }

        headers.insert(header::CONTENT_ENCODING, self.as_header_value());
        headers.remove(header::CONTENT_LENGTH);

        // Connection specific headers are not allowed in HTTP/2 and later versions.
        // see https://datatracker.ietf.org/doc/html/rfc7540#section-8.1.2.2
        if version == http::Version::HTTP_11 {
            headers.append(header::TRANSFER_ENCODING, header::HeaderValue::from_static("chunked"));
        }
    }

    fn _from_headers(headers: &HeaderMap, needle: &HeaderName) -> impl Iterator<Item = ContentEncodingWithQValue> {
        headers
            .get_all(needle)
            .iter()
            .filter_map(|hval| hval.to_str().ok())
            .flat_map(|s| s.split(','))
            .filter_map(|v| {
                let mut v = v.splitn(2, ';');
                Self::try_parse(v.next().unwrap().trim()).ok().map(|enc| {
                    let val = v
                        .next()
                        .and_then(|v| QValue::parse(v.trim()))
                        .unwrap_or_else(QValue::one);
                    ContentEncodingWithQValue { enc, val }
                })
            })
    }

    pub(super) fn try_parse(s: &str) -> Result<Self, FeatureError> {
        if s.eq_ignore_ascii_case("gzip") {
            Ok(Self::Gzip)
        } else if s.eq_ignore_ascii_case("deflate") {
            Ok(Self::Deflate)
        } else if s.eq_ignore_ascii_case("br") {
            Ok(Self::Br)
        } else if s.eq_ignore_ascii_case("zstd") {
            Ok(Self::Zstd)
        } else if s.eq_ignore_ascii_case("identity") {
            Ok(Self::Identity)
        } else {
            Err(FeatureError::Unknown(s.to_string().into_boxed_str()))
        }
    }
}

struct ContentEncodingWithQValue {
    enc: ContentEncoding,
    val: QValue,
}

impl Default for ContentEncodingWithQValue {
    fn default() -> Self {
        Self {
            enc: ContentEncoding::Identity,
            val: QValue::zero(),
        }
    }
}

impl ContentEncodingWithQValue {
    fn try_update(&mut self, other: Self) {
        if other.val > self.val {
            match other.enc {
                #[cfg(not(feature = "br"))]
                ContentEncoding::Br => return,
                #[cfg(not(feature = "de"))]
                ContentEncoding::Deflate => return,
                #[cfg(not(feature = "gz"))]
                ContentEncoding::Gzip => return,
                #[cfg(not(feature = "zs"))]
                ContentEncoding::Zstd => return,
                _ => {}
            };
            *self = other;
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct QValue(u16);

impl QValue {
    const fn zero() -> Self {
        Self(0)
    }

    const fn one() -> Self {
        Self(1000)
    }

    // Parse a q-value as specified in RFC 7231 section 5.3.1.
    fn parse(s: &str) -> Option<Self> {
        let mut c = s.chars();
        // Parse "q=" (case-insensitively).
        match c.next() {
            Some('q') | Some('Q') => (),
            _ => return None,
        };
        match c.next() {
            Some('=') => (),
            _ => return None,
        };

        // Parse leading digit. Since valid q-values are between 0.000 and 1.000, only "0" and "1"
        // are allowed.
        let mut value = match c.next() {
            Some('0') => 0,
            Some('1') => 1000,
            _ => return None,
        };

        // Parse optional decimal point.
        match c.next() {
            Some('.') => (),
            None => return Some(Self(value)),
            _ => return None,
        };

        // Parse optional fractional digits. The value of each digit is multiplied by `factor`.
        // Since the q-value is represented as an integer between 0 and 1000, `factor` is `100` for
        // the first digit, `10` for the next, and `1` for the digit after that.
        let mut factor = 100;
        loop {
            match c.next() {
                Some(n @ '0'..='9') => {
                    // If `factor` is less than `1`, three digits have already been parsed. A
                    // q-value having more than 3 fractional digits is invalid.
                    if factor < 1 {
                        return None;
                    }
                    // Add the digit's value multiplied by `factor` to `value`.
                    value += factor * (n as u16 - '0' as u16);
                }
                None => {
                    // No more characters to parse. Check that the value representing the q-value is
                    // in the valid range.
                    return if value <= 1000 { Some(Self(value)) } else { None };
                }
                _ => return None,
            };
            factor /= 10;
        }
    }
}
