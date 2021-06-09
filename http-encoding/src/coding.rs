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
}

impl Default for ContentEncoding {
    fn default() -> Self {
        Self::Identity
    }
}

impl From<&str> for ContentEncoding {
    fn from(_val: &str) -> ContentEncoding {
        #[cfg(any(feature = "br", feature = "gz", feature = "de"))]
        let _val = _val.trim();

        #[cfg(feature = "br")]
        if _val.eq_ignore_ascii_case("br") {
            return ContentEncoding::Br;
        }

        #[cfg(feature = "gz")]
        if _val.eq_ignore_ascii_case("gzip") {
            return ContentEncoding::Gzip;
        }

        #[cfg(feature = "de")]
        if _val.eq_ignore_ascii_case("deflate") {
            return ContentEncoding::Deflate;
        }

        ContentEncoding::default()
    }
}
