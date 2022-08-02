use std::{
    fmt,
    future::{poll_fn, Future},
    ops::{Deref, DerefMut},
};

use serde::{de::DeserializeOwned, ser::Serialize};
use xitca_unsafe_collection::pin;

use crate::{
    dev::bytes::{BufMutWriter, BytesMut},
    handler::{
        error::{ExtractError, _ParseError},
        FromRequest, Responder,
    },
    http::{const_header_value::JSON, header::CONTENT_TYPE},
    request::WebRequest,
    response::WebResponse,
    stream::WebStream,
};

use super::{
    body::Body,
    header::{self, HeaderRef},
};

const DEFAULT_LIMIT: usize = 1024 * 1024;

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
    B: WebStream,
    T: DeserializeOwned,
{
    type Type<'b> = Json<T, LIMIT>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async move {
            HeaderRef::<'a, { header::CONTENT_TYPE }>::from_request(req).await?;

            let limit = match HeaderRef::<'a, { header::CONTENT_LENGTH }>::from_request(req).await {
                Ok(header) => {
                    let len = header.try_parse()?;
                    std::cmp::min(len, LIMIT)
                }
                Err(_) => LIMIT,
            };

            let Body(body) = Body::from_request(req).await?;

            pin!(body);

            let mut buf = BytesMut::new();

            while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                let chunk = chunk.map_err(ExtractError::Body)?;
                let chunk = chunk.as_ref();
                if buf.len() + chunk.len() > limit {
                    panic!("error handling");
                }
                buf.extend_from_slice(chunk);
            }

            let json = serde_json::from_slice(&buf).map_err(_ParseError::JsonString)?;

            Ok(Json(json))
        }
    }
}

impl<'r, C, B, T, const LIMIT: usize> Responder<WebRequest<'r, C, B>> for Json<T, LIMIT>
where
    T: Serialize,
{
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    #[inline]
    fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
        let mut bytes = BytesMut::new();
        serde_json::to_writer(BufMutWriter(&mut bytes), &self.0).unwrap();
        let mut res = req.into_response(bytes.freeze());
        res.headers_mut().insert(CONTENT_TYPE, JSON);
        async { res }
    }
}
