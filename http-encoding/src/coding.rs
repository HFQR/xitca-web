use http::header::{HeaderMap, ACCEPT_ENCODING};

use super::error::FeatureError;

/// Represents a supported content encoding.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ContentEncoding {
    /// A format using the Brotli algorithm.
    Br,
    /// A format using the zlib structure with deflate algorithm.
    Deflate,
    /// Gzip algorithm.
    Gzip,
    /// Indicates no operation is done with encoding.
    #[default]
    NoOp,
}

impl ContentEncoding {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut preferred_encoding = Self::NoOp;
        let mut max_qval = 0;

        for (encoding, qval) in Self::_from_headers(headers) {
            if qval.0 > max_qval {
                preferred_encoding = encoding;
                max_qval = qval.0;
            }
        }

        preferred_encoding
    }

    fn _from_headers(headers: &HeaderMap) -> impl Iterator<Item = (Self, QValue)> + '_ {
        headers
            .get_all(ACCEPT_ENCODING)
            .iter()
            .filter_map(|hval| hval.to_str().ok())
            .flat_map(|s| s.split(','))
            .filter_map(|v| {
                let mut v = v.splitn(2, ';');

                let encoding = Self::try_parse(v.next().unwrap().trim())
                    // fallback to no-op encoding when preferred encoding is not supported.
                    .unwrap_or(ContentEncoding::NoOp);

                let qval = if let Some(qval) = v.next() {
                    QValue::parse(qval.trim())?
                } else {
                    QValue::one()
                };

                Some((encoding, qval))
            })
    }

    pub(super) fn try_parse(s: &str) -> Result<Self, FeatureError> {
        if s.eq_ignore_ascii_case("gzip") {
            Ok(Self::Gzip)
        } else if s.eq_ignore_ascii_case("deflate") {
            Ok(Self::Deflate)
        } else if s.eq_ignore_ascii_case("br") {
            Ok(Self::Br)
        } else if s.eq_ignore_ascii_case("identity") {
            Ok(Self::NoOp)
        } else {
            Err(FeatureError::Unknown(s.to_string().into_boxed_str()))
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct QValue(u16);

impl QValue {
    fn one() -> Self {
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
