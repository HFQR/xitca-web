//! type extractor and response generator for json

use core::{
    fmt,
    future::poll_fn,
    ops::{Deref, DerefMut},
    pin::pin,
};

use serde::{de::DeserializeOwned, ser::Serialize};

use crate::{
    body::BodyStream,
    bytes::{BufMutWriter, Bytes, BytesMut},
    handler::{error::ExtractError, FromRequest, Responder},
    http::{const_header_value::JSON, header::CONTENT_TYPE, status::StatusCode},
    request::WebRequest,
    response::WebResponse,
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

impl<'a, 'r, C, B, T, const LIMIT: usize> FromRequest<'a, WebRequest<'r, C, B>> for Json<T, LIMIT>
where
    B: BodyStream + Default,
    T: DeserializeOwned,
{
    type Type<'b> = Json<T, LIMIT>;
    type Error = ExtractError<B::Error>;

    async fn from_request(req: &'a WebRequest<'r, C, B>) -> Result<Self, Self::Error> {
        HeaderRef::<'a, { header::CONTENT_TYPE }>::from_request(req).await?;

        let limit = HeaderRef::<'a, { header::CONTENT_LENGTH }>::from_request(req)
            .await
            .ok()
            .and_then(|header| header.to_str().ok().and_then(|s| s.parse().ok()))
            .map(|len| std::cmp::min(len, LIMIT))
            .unwrap_or_else(|| LIMIT);

        let Body(body) = Body::from_request(req).await?;

        let mut body = pin!(body);

        let mut buf = BytesMut::new();

        while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            let chunk = chunk.map_err(ExtractError::Body)?;
            buf.extend_from_slice(chunk.as_ref());
            if buf.len() > limit {
                break;
            }
        }

        let json = serde_json::from_slice(&buf)?;

        Ok(Json(json))
    }
}

impl<'r, C, B, T> Responder<WebRequest<'r, C, B>> for Json<T>
where
    T: Serialize,
{
    type Output = WebResponse;

    #[inline]
    async fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Output {
        let mut bytes = BytesMut::new();
        match serde_json::to_writer(BufMutWriter(&mut bytes), &self.0) {
            Ok(_) => {
                let mut res = req.into_response(bytes.freeze());
                res.headers_mut().insert(CONTENT_TYPE, JSON);
                res
            }
            Err(_) => {
                let mut res = req.into_response(Bytes::new());
                *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                res
            }
        }
    }
}
