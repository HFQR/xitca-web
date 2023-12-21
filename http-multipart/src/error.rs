use std::{error, fmt};

#[derive(Debug)]
pub enum MultipartError {
    /// Only POST method is allowed for multipart.
    NoPostMethod,
    /// Content-Disposition header is not found or is not equal to "form-data".
    NoContentDisposition,
    /// Content-Type header is not found
    NoContentType,
    /// Can not parse Content-Type header
    ParseContentType,
    /// Multipart boundary is not found
    Boundary,
    /// Nested multipart is not supported
    Nested,
    /// Multipart stream is incomplete
    UnexpectedEof,
    /// Multipart parsing internal buffer overflown
    BufferOverflow,
    /// Error during header parsing
    Header(httparse::Error),
    /// Payload error
    Payload(PayloadError),
}

pub type PayloadError = Box<dyn error::Error + Send + Sync>;

impl fmt::Display for MultipartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NoPostMethod => f.write_str("Only POST method is allowed for multipart"),
            Self::NoContentDisposition => f.write_str("No Content-Disposition `form-data` header"),
            Self::NoContentType => f.write_str("No Content-Type header found"),
            Self::ParseContentType => f.write_str("Can not parse Content-Type header"),
            Self::Boundary => f.write_str("Multipart boundary is not found"),
            Self::Nested => f.write_str("Nested multipart is not supported"),
            Self::UnexpectedEof => f.write_str("Multipart stream ended early than expected."),
            Self::BufferOverflow => f.write_str("Multipart parsing internal buffer overflown"),
            Self::Header(ref e) => fmt::Display::fmt(e, f),
            Self::Payload(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl error::Error for MultipartError {}

impl From<httparse::Error> for MultipartError {
    fn from(e: httparse::Error) -> Self {
        Self::Header(e)
    }
}

impl From<PayloadError> for MultipartError {
    fn from(e: PayloadError) -> Self {
        Self::Payload(e)
    }
}
