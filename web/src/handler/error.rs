use core::{convert::Infallible, fmt};

use std::error;

use crate::{
    bytes::Bytes,
    context::WebContext,
    dev::service::Service,
    http::{StatusCode, WebResponse},
};

type BoxedError = Box<dyn error::Error + Send + Sync + 'static>;

pub(crate) fn blank_bad_request<C, B>(req: WebContext<'_, C, B>) -> WebResponse {
    let mut res = req.into_response(Bytes::new());
    *res.status_mut() = StatusCode::BAD_REQUEST;
    res
}

#[derive(Debug)]
pub struct Internal;

impl fmt::Display for Internal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Internal error")
    }
}

impl error::Error for Internal {}

impl<'r, C, B> Service<WebContext<'r, C, B>> for Internal {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(Bytes::new());
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        Ok(res)
    }
}

/// Collection of all default extract types's error.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExtractError {
    /// fallback boxed error type.
    Boxed(BoxedError),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Boxed(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl error::Error for ExtractError {}

impl From<Infallible> for ExtractError {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for ExtractError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(Bytes::new());
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        Ok(res)
    }
}

#[cfg(feature = "serde")]
impl<'r, C, B> Service<WebContext<'r, C, B>> for serde::de::value::Error {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Ok(blank_bad_request(ctx))
    }
}
