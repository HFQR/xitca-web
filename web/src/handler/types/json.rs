//! type extractor and response generator for json

use core::{
    convert::Infallible,
    fmt,
    future::poll_fn,
    ops::{Deref, DerefMut},
    pin::pin,
};

use serde::{de::DeserializeOwned, ser::Serialize};

use crate::{
    body::BodyStream,
    bytes::{BufMutWriter, BytesMut},
    context::WebContext,
    error::{BadRequest, Error},
    handler::{FromRequest, Responder},
    http::{const_header_value::JSON, header::CONTENT_TYPE, WebResponse},
    service::Service,
};

use super::{
    body::Body,
    header::{self, HeaderRef},
};

pub const DEFAULT_LIMIT: usize = 1024 * 1024;

/// Extract type for Json object. const generic param LIMIT is for max size of the object in bytes.
/// Object larger than limit would be treated as error.
///
/// Default limit is [DEFAULT_LIMIT] in bytes.
pub struct Json<T, const LIMIT: usize = DEFAULT_LIMIT>(pub T);

impl<T, const LIMIT: usize> fmt::Debug for Json<T, LIMIT>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Json")
            .field("value", &self.0)
            .field("limit", &LIMIT)
            .finish()
    }
}

impl<T, const LIMIT: usize> Deref for Json<T, LIMIT> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, const LIMIT: usize> DerefMut for Json<T, LIMIT> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a, 'r, C, B, T, const LIMIT: usize> FromRequest<'a, WebContext<'r, C, B>> for Json<T, LIMIT>
where
    B: BodyStream + Default,
    T: DeserializeOwned,
{
    type Type<'b> = Json<T, LIMIT>;
    type Error = Error<C>;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        HeaderRef::<'a, { header::CONTENT_TYPE }>::from_request(ctx).await?;

        let limit = HeaderRef::<'a, { header::CONTENT_LENGTH }>::from_request(ctx)
            .await
            .ok()
            .and_then(|header| header.to_str().ok().and_then(|s| s.parse().ok()))
            .map(|len| std::cmp::min(len, LIMIT))
            .unwrap_or_else(|| LIMIT);

        let Body(body) = Body::from_request(ctx).await?;

        let mut body = pin!(body);

        let mut buf = BytesMut::new();

        while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            let chunk = chunk.map_err(Into::into)?;
            buf.extend_from_slice(chunk.as_ref());
            if buf.len() > limit {
                break;
            }
        }

        serde_json::from_slice(&buf).map(Json).map_err(Into::into)
    }
}

impl<'r, C, B, T> Responder<WebContext<'r, C, B>> for Json<T>
where
    T: Serialize,
{
    type Response = WebResponse;
    type Error = Error<C>;

    #[inline]
    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut bytes = BytesMut::new();
        serde_json::to_writer(BufMutWriter(&mut bytes), &self.0)?;
        let mut res = ctx.into_response(bytes.freeze());
        res.headers_mut().insert(CONTENT_TYPE, JSON);
        Ok(res)
    }
}

impl<C> From<serde_json::Error> for Error<C> {
    fn from(e: serde_json::Error) -> Self {
        Self::from_service(e)
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for serde_json::Error {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        BadRequest.call(ctx).await
    }
}
