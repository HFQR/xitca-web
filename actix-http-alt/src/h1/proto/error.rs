#[derive(Debug)]
pub enum ProtoError {
    HttpParse(httparse::Error),
    Http(http::Error),
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
