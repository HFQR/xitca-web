use std::{error, fmt};

/// Error occur when trying to construct decode/encode request/response.
#[derive(Debug)]
#[non_exhaustive]
pub enum EncodingError {
    MissingFeature(FeatureError),
    ParseAcceptEncoding,
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MissingFeature(ref e) => write!(f, "{e}"),
            Self::ParseAcceptEncoding => write!(f, "failed to parse Accept-Encoding header value"),
        }
    }
}

/// Error for missing required feature.
#[derive(Debug)]
#[non_exhaustive]
pub enum FeatureError {
    Br,
    Gzip,
    Deflate,
    Unknown(Box<str>),
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Br => feature_error_fmt("brotil", f),
            Self::Gzip => feature_error_fmt("gzip", f),
            Self::Deflate => feature_error_fmt("deflate", f),
            Self::Unknown(ref encoding) => feature_error_fmt(encoding, f),
        }
    }
}

#[cold]
#[inline(never)]
fn feature_error_fmt(encoding: impl fmt::Display, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Content-Encoding: {encoding} is not supported.")
}

impl error::Error for FeatureError {}

impl From<FeatureError> for EncodingError {
    fn from(e: FeatureError) -> Self {
        Self::MissingFeature(e)
    }
}

/// Error occur when decode/encode request/response body stream.
pub type CoderError = Box<dyn error::Error + Send + Sync>;
