use std::{convert::Infallible, fmt, future::Future, string::FromUtf8Error};

use crate::{dev::bytes::Bytes, http::StatusCode, request::WebRequest, response::WebResponse};

use super::Responder;

#[derive(Debug)]
pub enum ExtractError {
    UnexpectedEof,
    ExtensionNotFound,
    HeaderNameNotFound,
    Parse(ParseError),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnexpectedEof => write!(f, "Request stream body ended unexpectedly"),
            Self::ExtensionNotFound => write!(f, "Extension can not be found"),
            Self::HeaderNameNotFound => write!(f, "HeaderName can not be found"),
            Self::Parse(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl From<Infallible> for ExtractError {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for ExtractError {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
        let mut res = req.into_response(Bytes::new());
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        async { res }
    }
}

#[derive(Debug)]
pub struct ParseError(_ParseError);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            _ParseError::String(ref e) => fmt::Display::fmt(e, f),
            #[cfg(feature = "json")]
            _ParseError::JsonString(ref e) => fmt::Display::fmt(e, f),
            #[cfg(feature = "json")]
            _ParseError::UrlEncoded(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

// a private type to hide 3rd part crates error types from ExtractError interface.
#[derive(Debug)]
pub(super) enum _ParseError {
    String(FromUtf8Error),
    #[cfg(feature = "json")]
    JsonString(serde_json::Error),
    #[cfg(feature = "urlencoded")]
    UrlEncoded(serde_urlencoded::de::Error),
}

impl From<_ParseError> for ExtractError {
    fn from(e: _ParseError) -> Self {
        Self::Parse(ParseError(e))
    }
}
