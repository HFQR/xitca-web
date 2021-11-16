use std::{cmp, error, fmt, str::FromStr};

/// Represents a supported content encoding.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ContentEncoding {
    /// Automatically select encoding based on encoding negotiation.
    Auto,

    /// A format using the Brotli algorithm.
    Br,

    /// A format using the zlib structure with deflate algorithm.
    Deflate,

    /// Gzip algorithm.
    Gzip,

    /// Indicates the identity function (i.e. no compression, nor modification).
    Identity,
}

impl ContentEncoding {
    /// Is the content compressed?
    #[inline]
    pub fn is_compression(self) -> bool {
        matches!(self, ContentEncoding::Identity | ContentEncoding::Auto)
    }

    /// Convert content encoding to string.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }

    /// Default Q-factor (quality) value.
    #[inline]
    pub fn quality(self) -> f64 {
        match self {
            ContentEncoding::Br => 1.1,
            ContentEncoding::Gzip => 1.0,
            ContentEncoding::Deflate => 0.9,
            ContentEncoding::Identity | ContentEncoding::Auto => 0.1,
        }
    }

    /// Return a file path extension str for according encoding variant.
    #[inline]
    pub fn as_path_extension(&self) -> &str {
        self.as_str()
    }
}

impl Default for ContentEncoding {
    fn default() -> Self {
        Self::Identity
    }
}

/// Error return when a content encoding is unknown.
///
/// Example: 'compress'
#[derive(Debug, PartialEq, Eq)]
pub enum EncodingError {
    Unsupported,
    Missing,
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unsupported => write!(f, "Unsupported encoding"),
            Self::Missing => write!(f, "Encoding missing"),
        }
    }
}

impl error::Error for EncodingError {}

impl FromStr for ContentEncoding {
    type Err = EncodingError;

    fn from_str(val: &str) -> Result<Self, Self::Err> {
        let val = val.trim();

        if val.eq_ignore_ascii_case("br") {
            Ok(ContentEncoding::Br)
        } else if val.eq_ignore_ascii_case("gzip") {
            Ok(ContentEncoding::Gzip)
        } else if val.eq_ignore_ascii_case("deflate") {
            Ok(ContentEncoding::Deflate)
        } else {
            Err(EncodingError::Unsupported)
        }
    }
}

impl TryFrom<&str> for ContentEncoding {
    type Error = EncodingError;

    fn try_from(val: &str) -> Result<Self, Self::Error> {
        val.parse()
    }
}

pub struct AcceptEncoding {
    pub encoding: ContentEncoding,
    // TODO: use Quality or QualityItem<ContentEncoding>
    quality: f64,
}

impl Eq for AcceptEncoding {}

impl Ord for AcceptEncoding {
    #[allow(clippy::comparison_chain)]
    fn cmp(&self, other: &AcceptEncoding) -> cmp::Ordering {
        if self.quality > other.quality {
            cmp::Ordering::Less
        } else if self.quality < other.quality {
            cmp::Ordering::Greater
        } else {
            cmp::Ordering::Equal
        }
    }
}

impl PartialOrd for AcceptEncoding {
    fn partial_cmp(&self, other: &AcceptEncoding) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for AcceptEncoding {
    fn eq(&self, other: &AcceptEncoding) -> bool {
        self.encoding == other.encoding && self.quality == other.quality
    }
}

/// Parse q-factor from quality strings.
///
/// If parse fail, then fallback to default value which is 1.
/// More details available here: <https://developer.mozilla.org/en-US/docs/Glossary/Quality_values>
fn parse_quality<'a>(parts: impl Iterator<Item = &'a str>) -> f64 {
    for part in parts {
        if part.trim().starts_with("q=") {
            return part[2..].parse().unwrap_or(1.0);
        }
    }

    1.0
}

impl AcceptEncoding {
    pub fn new(tag: &str) -> Option<AcceptEncoding> {
        let mut parts = tag.split(';');

        parts
            .next()
            .and_then(|part| ContentEncoding::try_from(part).ok())
            .and_then(|encoding| {
                let quality = parse_quality(parts);
                (0.0 < quality && quality <= 1.0).then(|| AcceptEncoding { encoding, quality })
            })
    }

    #[inline]
    pub fn encoding(&self) -> &ContentEncoding {
        &self.encoding
    }

    /// Parse a raw Accept-Encoding header value into an ordered list then return the best match
    /// based on middleware configuration.
    pub fn try_parse(raw: &str, encoding: ContentEncoding) -> Result<ContentEncoding, EncodingError> {
        let mut encodings = raw
            .replace(' ', "")
            .split(',')
            .filter_map(AcceptEncoding::new)
            .collect::<Vec<_>>();

        encodings.sort();

        for enc in encodings {
            if encoding == ContentEncoding::Auto || encoding == enc.encoding {
                return Ok(enc.encoding);
            }
        }

        // Special case if user cannot accept uncompressed data.
        // See: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Accept-Encoding
        // TODO: account for whitespace
        if raw.contains("*;q=0") || raw.contains("identity;q=0") {
            return Err(EncodingError::Missing);
        }

        Ok(ContentEncoding::Identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! assert_parse_eq {
        ($raw:expr, $result:expr) => {
            assert_eq!(
                AcceptEncoding::try_parse($raw, ContentEncoding::Auto),
                Ok($result)
            );
        };
    }

    macro_rules! assert_parse_fail {
        ($raw:expr) => {
            assert!(AcceptEncoding::try_parse($raw, ContentEncoding::Auto).is_err());
        };
    }

    #[test]
    fn test_parse_encoding() {
        // Test simple case
        assert_parse_eq!("br", ContentEncoding::Br);
        assert_parse_eq!("gzip", ContentEncoding::Gzip);
        assert_parse_eq!("deflate", ContentEncoding::Deflate);

        // Test space, trim, missing values
        assert_parse_eq!("br,,,,", ContentEncoding::Br);
        assert_parse_eq!("gzip  ,   br,   zstd", ContentEncoding::Gzip);

        // Test float number parsing
        assert_parse_eq!("br;q=1  ,", ContentEncoding::Br);
        assert_parse_eq!("br;q=1.0  ,   br", ContentEncoding::Br);

        // Test wildcard
        assert_parse_eq!("*", ContentEncoding::Identity);
        assert_parse_eq!("*;q=1.0", ContentEncoding::Identity);
    }

    #[test]
    fn test_parse_encoding_qfactor_ordering() {
        assert_parse_eq!("gzip, br, zstd", ContentEncoding::Gzip);

        assert_parse_eq!("gzip;q=0.4, br;q=0.6", ContentEncoding::Br);
        assert_parse_eq!("gzip;q=0.8, br;q=0.4", ContentEncoding::Gzip);
    }

    #[test]
    fn test_parse_encoding_qfactor_invalid() {
        // Out of range
        assert_parse_eq!("gzip;q=-5.0", ContentEncoding::Identity);
        assert_parse_eq!("gzip;q=5.0", ContentEncoding::Identity);

        // Disabled
        assert_parse_eq!("gzip;q=0", ContentEncoding::Identity);
    }

    #[test]
    fn test_parse_compression_required() {
        // Check we fallback to identity if there is an unsupported compression algorithm
        assert_parse_eq!("compress", ContentEncoding::Identity);

        // User do not want any compression
        assert_parse_fail!("compress, identity;q=0");
        assert_parse_fail!("compress, identity;q=0.0");
        assert_parse_fail!("compress, *;q=0");
        assert_parse_fail!("compress, *;q=0.0");
    }
}
