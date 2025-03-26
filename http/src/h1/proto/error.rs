use httparse::Error as HttparseError;

#[derive(Debug)]
pub enum ProtoError {
    HeaderName,
    HeaderValue,
    HeaderTooLarge,
    Method,
    Uri,
    NewLine,
    Status,
    Token,
    Version,
}

impl From<HttparseError> for ProtoError {
    fn from(e: HttparseError) -> Self {
        match e {
            HttparseError::HeaderName => Self::HeaderName,
            HttparseError::HeaderValue => Self::HeaderValue,
            // Too many headers would be treated the same as header too large error.
            HttparseError::TooManyHeaders => Self::HeaderTooLarge,
            HttparseError::NewLine => Self::NewLine,
            HttparseError::Status => Self::Status,
            HttparseError::Token => Self::Token,
            HttparseError::Version => Self::Version,
        }
    }
}

impl From<http::method::InvalidMethod> for ProtoError {
    fn from(_: http::method::InvalidMethod) -> Self {
        Self::Method
    }
}

impl From<http::uri::InvalidUri> for ProtoError {
    fn from(_: http::uri::InvalidUri) -> Self {
        Self::Uri
    }
}

impl From<http::uri::InvalidUriParts> for ProtoError {
    fn from(_: http::uri::InvalidUriParts) -> Self {
        Self::Uri
    }
}

impl From<http::status::InvalidStatusCode> for ProtoError {
    fn from(_: http::status::InvalidStatusCode) -> Self {
        Self::Status
    }
}
