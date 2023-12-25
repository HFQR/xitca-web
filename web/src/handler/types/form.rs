use core::{
    cmp, fmt,
    future::poll_fn,
    ops::{Deref, DerefMut},
    pin::pin,
};

use serde::{de::DeserializeOwned, Serialize};

use crate::{
    body::BodyStream,
    bytes::{Bytes, BytesMut},
    error::{error_from_service, forward_blank_bad_request, Error},
    handler::{
        body::Body,
        header::{self, HeaderRef},
        FromRequest, Responder,
    },
    http::{const_header_value::APPLICATION_WWW_FORM_URLENCODED, header::CONTENT_TYPE, WebResponse},
    WebContext,
};

pub const DEFAULT_LIMIT: usize = 1024 * 1024;

/// Extract type for form object. const generic param LIMIT is for max size of the object in bytes.
/// Object larger than limit would be treated as error.
///
/// Default limit is [DEFAULT_LIMIT] in bytes.
pub struct Form<T, const LIMIT: usize = DEFAULT_LIMIT>(pub T);

impl<T, const LIMIT: usize> fmt::Debug for Form<T, LIMIT>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Form")
            .field("value", &self.0)
            .field("limit", &LIMIT)
            .finish()
    }
}

impl<T, const LIMIT: usize> Deref for Form<T, LIMIT> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, const LIMIT: usize> DerefMut for Form<T, LIMIT> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a, 'r, C, B, T, const LIMIT: usize> FromRequest<'a, WebContext<'r, C, B>> for Form<T, LIMIT>
where
    B: BodyStream + Default,
    T: DeserializeOwned,
{
    type Type<'b> = Form<T, LIMIT>;
    type Error = Error<C>;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        HeaderRef::<'a, { header::CONTENT_TYPE }>::from_request(ctx).await?;

        let limit = HeaderRef::<'a, { header::CONTENT_LENGTH }>::from_request(ctx)
            .await
            .ok()
            .and_then(|header| header.to_str().ok().and_then(|s| s.parse().ok()))
            .map(|len| cmp::min(len, LIMIT))
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

        serde_urlencoded::from_bytes(&buf).map(Form).map_err(Into::into)
    }
}

impl<'r, C, B, T> Responder<WebContext<'r, C, B>> for Form<T>
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

impl<T> Form<T> {
    fn _respond<F, C>(self, func: F) -> Result<WebResponse, Error<C>>
    where
        T: Serialize,
        F: FnOnce(Bytes) -> WebResponse,
    {
        let string = serde_urlencoded::to_string(self.0)?;
        let mut res = func(Bytes::from(string));
        res.headers_mut().insert(CONTENT_TYPE, APPLICATION_WWW_FORM_URLENCODED);
        Ok(res)
    }
}

error_from_service!(serde_urlencoded::ser::Error);
forward_blank_bad_request!(serde_urlencoded::ser::Error);

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{http::header::CONTENT_LENGTH, test::collect_body};

    use super::*;

    #[derive(serde::Deserialize, serde::Serialize)]
    struct Student {
        name: String,
        age: u8,
    }

    #[test]
    fn extract_and_respond() {
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();

        let body: &[u8] = b"name=arisu&age=14";

        ctx.req_mut()
            .headers_mut()
            .insert(CONTENT_TYPE, APPLICATION_WWW_FORM_URLENCODED);

        ctx.req_mut()
            .headers_mut()
            .insert(CONTENT_LENGTH, body.len().try_into().unwrap());

        *ctx.body_borrow_mut() = body.into();

        let Form(s) = Form::<Student>::from_request(&ctx).now_or_panic().unwrap();

        assert_eq!(s.name, "arisu");
        assert_eq!(s.age, 14);

        let res = Form(s).respond(ctx).now_or_panic().unwrap();
        assert_eq!(res.status().as_u16(), 200);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            APPLICATION_WWW_FORM_URLENCODED
        );
        let body2 = collect_body(res.into_body()).now_or_panic().unwrap();
        assert_eq!(body2.as_slice(), body);
    }
}
