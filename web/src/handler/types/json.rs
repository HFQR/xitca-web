//! type extractor and response generator for json

use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use serde::{de::Deserialize, ser::Serialize};

use crate::{
    body::BodyStream,
    bytes::{BufMutWriter, Bytes, BytesMut},
    context::WebContext,
    error::{error_from_service, forward_blank_bad_request, Error},
    handler::{FromRequest, Responder},
    http::{const_header_value::JSON, header::CONTENT_TYPE, WebResponse},
};

use super::{
    body::Limit,
    header::{self, HeaderRef},
    lazy::Lazy,
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
    T: for<'de> Deserialize<'de>,
{
    type Type<'b> = Json<T, LIMIT>;
    type Error = Error<C>;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        HeaderRef::<'a, { header::CONTENT_TYPE }>::from_request(ctx).await?;
        let (bytes, _) = <(BytesMut, Limit<LIMIT>)>::from_request(ctx).await?;
        serde_json::from_slice(&bytes).map(Json).map_err(Into::into)
    }
}

impl<T, const LIMIT: usize> Lazy<Json<T, LIMIT>> {
    pub fn deserialize<'de, C>(&'de self) -> Result<Json<T, LIMIT>, Error<C>>
    where
        T: Deserialize<'de>,
    {
        serde_json::from_slice(&self.buf).map(Json).map_err(Into::into)
    }
}

impl<'a, 'r, C, B, T, const LIMIT: usize> FromRequest<'a, WebContext<'r, C, B>> for Lazy<Json<T, LIMIT>>
where
    B: BodyStream + Default,
    T: Deserialize<'static>,
{
    type Type<'b> = Lazy<Json<T, LIMIT>>;
    type Error = Error<C>;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        HeaderRef::<'a, { header::CONTENT_TYPE }>::from_request(ctx).await?;
        let (bytes, _) = <(Vec<u8>, Limit<LIMIT>)>::from_request(ctx).await?;
        Ok(Lazy::new(bytes))
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
        self._respond(|bytes| ctx.into_response(bytes))
    }

    #[inline]
    fn map(self, res: Self::Response) -> Result<Self::Response, Self::Error> {
        self._respond(|bytes| res.map(|_| bytes.into()))
    }
}

impl<T> Json<T> {
    fn _respond<F, C>(self, func: F) -> Result<WebResponse, Error<C>>
    where
        T: Serialize,
        F: FnOnce(Bytes) -> WebResponse,
    {
        let mut bytes = BytesMut::new();
        serde_json::to_writer(BufMutWriter(&mut bytes), &self.0)?;
        let mut res = func(bytes.freeze());
        res.headers_mut().insert(CONTENT_TYPE, JSON);
        Ok(res)
    }
}

error_from_service!(serde_json::Error);
forward_blank_bad_request!(serde_json::Error);

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::http::header::CONTENT_LENGTH;

    use super::*;

    #[derive(serde::Deserialize, serde::Serialize)]
    struct Gacha<'a> {
        name: &'a str,
    }

    #[test]
    fn extract_lazy() {
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();

        let body = serde_json::to_string(&Gacha { name: "arisu" }).unwrap();

        ctx.req_mut().headers_mut().insert(CONTENT_TYPE, JSON);
        ctx.req_mut().headers_mut().insert(CONTENT_LENGTH, body.len().into());

        *ctx.body_borrow_mut() = body.into();

        let lazy = Lazy::<Json<Gacha<'_>>>::from_request(&ctx).now_or_panic().unwrap();

        let Json(user) = lazy.deserialize::<()>().unwrap();

        assert_eq!(user.name, "arisu");
    }
}
