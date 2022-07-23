/// Represents a supported content encoding.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ContentEncoding {
    /// A format using the Brotli algorithm.
    Br,
    /// A format using the zlib structure with deflate algorithm.
    Deflate,
    /// Gzip algorithm.
    Gzip,
    /// Indicates no operation is done with encoding.
    NoOp,
}

impl ContentEncoding {
    /// Is the content compressed?
    #[inline]
    pub fn is_compression(self) -> bool {
        !matches!(self, ContentEncoding::NoOp)
    }

    /// Default Q-factor (quality) value.
    #[inline]
    pub fn quality(self) -> f64 {
        match self {
            ContentEncoding::Br => 1.1,
            ContentEncoding::Gzip => 1.0,
            ContentEncoding::Deflate => 0.9,
            ContentEncoding::NoOp => 0.1,
        }
    }
}

impl Default for ContentEncoding {
    fn default() -> Self {
        Self::NoOp
    }
}

impl From<&str> for ContentEncoding {
    fn from(val: &str) -> ContentEncoding {
        let val = val.trim();

        if val.eq_ignore_ascii_case("br") {
            ContentEncoding::Br
        } else if val.eq_ignore_ascii_case("gzip") {
            ContentEncoding::Gzip
        } else if val.eq_ignore_ascii_case("deflate") {
            ContentEncoding::Deflate
        } else {
            ContentEncoding::default()
        }
    }
}
