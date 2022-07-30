use std::{error, fmt, io};

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
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Br => write!(f, "br feature is disabled."),
            Self::Gzip => write!(f, "gz feature is disabled."),
            Self::Deflate => write!(f, "de feature is disabled."),
        }
    }
}

impl error::Error for FeatureError {}

impl From<FeatureError> for EncodingError {
    fn from(e: FeatureError) -> Self {
        Self::MissingFeature(e)
    }
}

/// Error occur when decode/encode request/response body stream.
pub enum CoderError<E> {
    Io(io::Error),
    Stream(E),
}

impl<E> fmt::Debug for CoderError<E>
where
    E: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{:?}", e),
            Self::Stream(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl<E> fmt::Display for CoderError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{e}"),
            Self::Stream(ref e) => write!(f, "{e}"),
        }
    }
}

impl<E> error::Error for CoderError<E> where E: fmt::Debug + fmt::Display {}

impl<E> From<io::Error> for CoderError<E> {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
