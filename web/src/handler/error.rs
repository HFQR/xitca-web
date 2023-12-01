use core::{convert::Infallible, fmt, str::Utf8Error};

use std::error;

#[cfg(feature = "multipart")]
use http_multipart::MultipartError;
#[cfg(feature = "json")]
use serde_json::Error as JsonError;

use crate::{
    bytes::Bytes,
    error::BodyError,
    http::{header::HeaderName, StatusCode},
    request::WebRequest,
    response::WebResponse,
};

use super::Responder;

type BoxedError = Box<dyn error::Error + Send + Sync + 'static>;

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
    Boxed(BoxedError),
}

impl<E: fmt::Display> fmt::Display for ExtractError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Body(ref e) => fmt::Display::fmt(e, f),
            Self::ExtensionNotFound => f.write_str("Extension can not be found"),
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
            #[cfg(feature = "multipart")]
            _ParseError::Multipart(ref e) => fmt::Display::fmt(e, f),
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
    JsonString(JsonError),
    #[cfg(feature = "urlencoded")]
    UrlEncoded(serde_urlencoded::de::Error),
    #[cfg(feature = "multipart")]
    Multipart(MultipartError<Infallible>),
}

impl<E> From<_ParseError> for ExtractError<E> {
    fn from(e: _ParseError) -> Self {
        Self::Parse(ParseError(e))
    }
}

#[cfg(feature = "json")]
impl<E> From<JsonError> for ExtractError<E> {
    fn from(e: JsonError) -> Self {
        Self::from(_ParseError::JsonString(e))
    }
}

#[cfg(feature = "multipart")]
impl<E> From<MultipartError<E>> for ExtractError<E> {
    fn from(e: MultipartError<E>) -> Self {
        // TODO: sort this out?
        match e {
            MultipartError::NoPostMethod => Self::from(_ParseError::Multipart(MultipartError::NoPostMethod)),
            MultipartError::NoContentDisposition => {
                Self::from(_ParseError::Multipart(MultipartError::NoContentDisposition))
            }
            MultipartError::NoContentType => Self::from(_ParseError::Multipart(MultipartError::NoContentType)),
            MultipartError::ParseContentType => Self::from(_ParseError::Multipart(MultipartError::ParseContentType)),
            MultipartError::Boundary => Self::from(_ParseError::Multipart(MultipartError::Boundary)),
            MultipartError::Nested => Self::from(_ParseError::Multipart(MultipartError::Nested)),
            MultipartError::UnexpectedEof => Self::from(_ParseError::Multipart(MultipartError::UnexpectedEof)),
            MultipartError::BufferOverflow => Self::from(_ParseError::Multipart(MultipartError::BufferOverflow)),
            MultipartError::Header(e) => Self::from(_ParseError::Multipart(MultipartError::Header(e))),
            MultipartError::Payload(e) => Self::Body(e),
        }
    }
}
