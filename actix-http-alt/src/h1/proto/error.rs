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
    Header,
    StatusCode,
    HeaderValue,
}

impl From<httparse::Error> for ProtoError {
    fn from(e: httparse::Error) -> Self {
        Self::HttpParse(e)
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
