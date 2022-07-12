use std::{error, fmt};

#[derive(Debug)]
pub enum MultipartError<E> {
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
    Incomplete,
    /// Error during header parsing
    Header(httparse::Error),
    /// Payload error
    Payload(E),
    /// Not consumed
    NotConsumed,
}

impl<E> fmt::Display for MultipartError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NoContentDisposition => write!(f, "No Content-Disposition `form-data` header"),
            Self::NoContentType => write!(f, "No Content-Type header found"),
            Self::ParseContentType => write!(f, "Can not parse Content-Type header"),
            Self::Boundary => write!(f, "Multipart boundary is not found"),
            Self::Nested => write!(f, "Nested multipart is not supported"),
            Self::Incomplete => write!(f, "Multipart stream is incomplete"),
            Self::Header(ref e) => write!(f, "{}", e),
            Self::Payload(ref e) => write!(f, "{}", e),
            Self::NotConsumed => write!(f, "Multipart stream is not consumed"),
        }
    }
}

impl<E> error::Error for MultipartError<E> where E: fmt::Debug + fmt::Display {}

impl<E> From<httparse::Error> for MultipartError<E> {
    fn from(e: httparse::Error) -> Self {
        Self::Header(e)
    }
}
