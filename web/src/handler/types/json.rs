use std::{
    convert::Infallible,
    fmt,
    future::Future,
    ops::{Deref, DerefMut},
};

use futures_util::StreamExt;
use serde::{de::DeserializeOwned, ser::Serialize};

use crate::{
    dev::bytes::{BufMutWriter, BytesMut},
    handler::{Extract, Inject},
    http::{const_header_value::JSON, header::CONTENT_TYPE},
    request::WebRequest,
    response::WebResponse,
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

impl<'a, 'r, S, T, const LIMIT: usize> Extract<'a, WebRequest<'r, S>> for Json<T, LIMIT>
where
    S: 'r,
    T: DeserializeOwned,
{
    type Type<'b> = Json<T, LIMIT>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    fn extract(req: &'a WebRequest<'r, S>) -> Self::Future {
        async move {
            HeaderRef::<'a, { header::CONTENT_TYPE }>::extract(req).await?;

            let limit = match HeaderRef::<'a, { header::CONTENT_LENGTH }>::extract(req).await {
                Ok(header) => {
                    let len = header.try_parse()?;
                    std::cmp::min(len, LIMIT)
                }
                Err(_) => LIMIT,
            };

            let Body(mut body) = Body::extract(req).await?;

            let mut buf = BytesMut::new();

            while let Some(Ok(chunk)) = body.next().await {
                if buf.len() + chunk.len() >= limit {
                    panic!("error handling");
                }
                buf.extend_from_slice(&chunk);
            }

            let json = serde_json::from_slice(&buf).expect("error handling is to do");

            Ok(Json(json))
        }
    }
}

impl<'r, S, T, const LIMIT: usize> Inject<WebRequest<'r, S>> for Json<T, LIMIT>
where
    S: 'r,
    T: Serialize,
{
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    #[inline]
    fn inject(self, req: WebRequest<'r, S>) -> Self::Future {
        let mut bytes = BytesMut::new();
        serde_json::to_writer(BufMutWriter(&mut bytes), &self.0).unwrap();
        let mut res = req.into_response(bytes.freeze());
        res.headers_mut().insert(CONTENT_TYPE, JSON);
        async { res }
    }
}
