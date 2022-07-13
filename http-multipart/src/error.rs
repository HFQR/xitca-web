use std::{error, fmt};

#[derive(Debug, Eq, PartialEq)]
pub enum MultipartError<E> {
    /// Only POST method is allowed for multipart.
    NoPostMethod,
    /// Content-Disposition header is not found or is not equal to "form-data".
    ///
    /// According to [RFC 7578 §4.2](https://datatracker.ietf.org/doc/html/rfc7578#section-4.2) a
    /// Content-Disposition header must always be present and equal to "form-data".
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
    /// Error during header parsing
    Header(httparse::Error),
    /// Payload error
    Payload(E),
}

impl<E> fmt::Display for MultipartError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NoPostMethod => write!(f, "Only POST method is allowed for multipart"),
            Self::NoContentDisposition => write!(f, "No Content-Disposition `form-data` header"),
            Self::NoContentType => write!(f, "No Content-Type header found"),
            Self::ParseContentType => write!(f, "Can not parse Content-Type header"),
            Self::Boundary => write!(f, "Multipart boundary is not found"),
            Self::Nested => write!(f, "Nested multipart is not supported"),
            Self::UnexpectedEof => write!(f, "Multipart stream ended early than expected."),
            Self::Header(ref e) => write!(f, "{}", e),
            Self::Payload(ref e) => write!(f, "{}", e),
        }
    }
}

impl<E> error::Error for MultipartError<E> where E: fmt::Debug + fmt::Display {}

impl<E> From<httparse::Error> for MultipartError<E> {
    fn from(e: httparse::Error) -> Self {
        Self::Header(e)
    }
}
