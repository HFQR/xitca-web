use std::{convert::Infallible, error, fmt, str::Utf8Error};

use crate::{
    dev::bytes::Bytes,
    error::BodyError,
    http::{header::HeaderName, StatusCode},
    request::WebRequest,
    response::WebResponse,
};

use super::Responder;

/// Collection of all default extract types's error.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExtractError<E = BodyError> {
    /// Request body error.
    Body(E),
    /// Absent type of request's (Extensions)[crate::http::Extensions] type map.
    ExtensionNotFound,
    /// Absent header value.
    HeaderNotFound(HeaderName),
    /// Error of parsing bytes to Rust types.
    Parse(ParseError),
    /// fallback boxed error type.
    Boxed(Box<dyn error::Error + Send + Sync + 'static>),
}

impl<E: fmt::Display> fmt::Display for ExtractError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Body(ref e) => fmt::Display::fmt(e, f),
            Self::ExtensionNotFound => write!(f, "Extension can not be found"),
            Self::HeaderNotFound(ref name) => write!(f, "HeaderName: {name} not found."),
            Self::Parse(ref e) => fmt::Display::fmt(e, f),
            Self::Boxed(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl<E> error::Error for ExtractError<E> where E: fmt::Debug + fmt::Display {}

impl<E> From<Infallible> for ExtractError<E> {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl<'r, C, B, E> Responder<WebRequest<'r, C, B>> for ExtractError<E> {
    type Output = WebResponse;

    async fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Output {
        let mut res = req.into_response(Bytes::new());
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res
    }
}

#[derive(Debug)]
pub struct ParseError(_ParseError);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            _ParseError::String(ref e) => fmt::Display::fmt(e, f),
            #[cfg(feature = "params")]
            _ParseError::Params(ref e) => fmt::Display::fmt(e, f),
            #[cfg(feature = "json")]
            _ParseError::JsonString(ref e) => fmt::Display::fmt(e, f),
            #[cfg(feature = "urlencoded")]
            _ParseError::UrlEncoded(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

// a private type to hide 3rd part crates error types from ExtractError interface.
#[derive(Debug)]
pub(super) enum _ParseError {
    String(Utf8Error),
    #[cfg(feature = "params")]
    Params(serde::de::value::Error),
    #[cfg(feature = "json")]
    JsonString(serde_json::Error),
    #[cfg(feature = "urlencoded")]
    UrlEncoded(serde_urlencoded::de::Error),
}

impl<E> From<_ParseError> for ExtractError<E> {
    fn from(e: _ParseError) -> Self {
        Self::Parse(ParseError(e))
    }
}
