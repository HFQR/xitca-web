use httparse::Error as HttparseError;

#[derive(Debug)]
pub enum ProtoError {
    // crate level parse error.
    Parse(Parse),
    // error from httparse crate.
    HttpParse(httparse::Error),
    // error from http crate.
    Http(http::Error),
}

/// Failure on parsing.
#[derive(Debug)]
pub enum Parse {
    HeaderName,
    HeaderValue,
    HeaderTooLarge,
    StatusCode,
}

impl From<HttparseError> for ProtoError {
    fn from(e: HttparseError) -> Self {
        match e {
            // Too many headers would be treated the same as header too large to handle.
            // This is caused by overflow of HttpServiceConfig's MAX_HEADERS const generic
            // usize.
            HttparseError::TooManyHeaders => Self::Parse(Parse::HeaderTooLarge),
            HttparseError::HeaderName => Self::Parse(Parse::HeaderName),
            HttparseError::HeaderValue => Self::Parse(Parse::HeaderValue),
            e => Self::HttpParse(e),
        }
    }
}

impl From<http::Error> for ProtoError {
    fn from(e: http::Error) -> Self {
        Self::Http(e)
    }
}

impl From<http::method::InvalidMethod> for ProtoError {
    fn from(e: http::method::InvalidMethod) -> Self {
        Self::Http(e.into())
    }
}

impl From<http::uri::InvalidUri> for ProtoError {
    fn from(e: http::uri::InvalidUri) -> Self {
        Self::Http(e.into())
    }
}

impl From<Parse> for ProtoError {
    fn from(e: Parse) -> Self {
        Self::Parse(e)
    }
}
