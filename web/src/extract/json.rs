use std::{
    convert::Infallible,
    fmt,
    future::Future,
    ops::{Deref, DerefMut},
};

use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use xitca_http::util::service::FromRequest;
use xitca_io::bytes::BytesMut;

use crate::request::WebRequest;

use super::{Body, HeaderName, HeaderRef};

/// Extract type for Json object. const generic param LIMIT is for max size of the object in bytes.
/// Object larger than limit would be treated as error.
pub struct Json<T, const LIMIT: usize>(pub T);

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

impl<'a, 'r, 's, S, T, const LIMIT: usize> FromRequest<'a, &'r mut WebRequest<'s, S>> for Json<T, LIMIT>
where
    T: DeserializeOwned,
{
    type Type<'b> = Json<T, LIMIT>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move {
            // TODO: check if is json value.
            HeaderRef::<'a, { HeaderName::CONTENT_TYPE }>::from_request(req).await?;

            let limit = match HeaderRef::<'a, { HeaderName::CONTENT_LENGTH }>::from_request(req).await {
                Ok(header) => {
                    let len = header.try_parse()?;
                    std::cmp::min(len, LIMIT)
                }
                Err(_) => LIMIT,
            };

            let Body(mut body) = Body::from_request(req).await?;

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
