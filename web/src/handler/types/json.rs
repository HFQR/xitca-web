//! type extractor and response generator for json

use core::{
    convert::Infallible,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use serde::{de::Deserialize, ser::Serialize};
use xitca_http::util::service::router::{PathGen, RouteGen, RouterMapErr};

use crate::{
    body::BodyStream,
    bytes::{BufMutWriter, Bytes, BytesMut},
    context::WebContext,
    error::{Error, error_from_service, forward_blank_bad_request},
    handler::{FromRequest, Responder},
    http::{WebResponse, const_header_value::JSON, header::CONTENT_TYPE},
    service::Service,
};

use super::{
    body::Limit,
    header::{self, HeaderRef},
};

pub const DEFAULT_LIMIT: usize = 1024 * 1024;

/// Extract type for Json object. const generic param LIMIT is for max size of the object in bytes.
/// Object larger than limit would be treated as error.
///
/// Default limit is [DEFAULT_LIMIT] in bytes.
#[derive(Clone)]
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
    type Error = Error;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        HeaderRef::<'a, { header::CONTENT_TYPE }>::from_request(ctx).await?;
        let (bytes, _) = <(BytesMut, Limit<LIMIT>)>::from_request(ctx).await?;
        serde_json::from_slice(&bytes).map(Json).map_err(Into::into)
    }
}

/// lazy deserialize type that wrap around [Json]. It lowers the deserialization to handler
/// function where zero copy deserialize can happen.
///
/// # Example
/// ```rust
/// # use serde::Deserialize;
/// # use xitca_web::{
/// #   error::Error,
/// #   http::StatusCode,
/// #   handler::{handler_service, json::LazyJson},
/// #   App, WebContext
/// # };
/// // a json object with zero copy deserialization.
/// #[derive(Deserialize)]
/// struct Post<'a> {
///     title: &'a str,
///     content: &'a str    
/// }
///
/// // handler function utilize Lazy type to lower the Json type into handler function.
/// async fn handler(lazy: LazyJson<Post<'_>>) -> Result<String, Error> {
///     // actual deserialize happens here.
///     let Post { title, content } = lazy.deserialize()?;
///     // the Post type and it's &str referencing LazyJson would live until the handler
///     // function return.
///     Ok(format!("Post {{ title: {title}, content: {content} }}"))
/// }
///
/// App::new()
///     .at("/post", handler_service(handler))
///     # .at("/", handler_service(|_: &WebContext<'_>| async { "used for infer type" }));
/// ```
pub struct LazyJson<T, const LIMIT: usize = DEFAULT_LIMIT> {
    bytes: Vec<u8>,
    _json: PhantomData<T>,
}

impl<T, const LIMIT: usize> LazyJson<T, LIMIT> {
    pub fn deserialize<'de>(&'de self) -> Result<T, Error>
    where
        T: Deserialize<'de>,
    {
        serde_json::from_slice(&self.bytes).map_err(Into::into)
    }
}

impl<'a, 'r, C, B, T, const LIMIT: usize> FromRequest<'a, WebContext<'r, C, B>> for LazyJson<T, LIMIT>
where
    B: BodyStream + Default,
    T: Deserialize<'static>,
{
    type Type<'b> = LazyJson<T, LIMIT>;
    type Error = Error;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        HeaderRef::<'a, { header::CONTENT_TYPE }>::from_request(ctx).await?;
        let (bytes, _) = <(Vec<u8>, Limit<LIMIT>)>::from_request(ctx).await?;
        Ok(LazyJson {
            bytes,
            _json: PhantomData,
        })
    }
}

impl<'r, C, B, T> Responder<WebContext<'r, C, B>> for Json<T>
where
    T: Serialize,
{
    type Response = WebResponse;
    type Error = Error;

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
    fn _respond<F>(self, func: F) -> Result<WebResponse, Error>
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

impl<'r, C, B> Responder<WebContext<'r, C, B>> for serde_json::Value {
    type Response = WebResponse;
    type Error = Error;

    #[inline]
    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Json(self).respond(ctx).await
    }

    #[inline]
    fn map(self, res: Self::Response) -> Result<Self::Response, Self::Error> {
        Responder::<WebContext<'r, C, B>>::map(Json(self), res)
    }
}

error_from_service!(serde_json::Error);
forward_blank_bad_request!(serde_json::Error);

impl<T> PathGen for Json<T> {}

impl<T> RouteGen for Json<T> {
    type Route<R> = RouterMapErr<R>;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        RouterMapErr(route)
    }
}

impl<T> Service for Json<T>
where
    T: Clone,
{
    type Response = Self;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        Ok(self.clone())
    }
}

impl<'r, C, B, T> Service<WebContext<'r, C, B>> for Json<T>
where
    T: Serialize + Clone,
{
    type Response = WebResponse;
    type Error = Error;

    #[inline]
    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.clone().respond(ctx).await
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        App,
        handler::handler_service,
        http::{WebRequest, header::CONTENT_LENGTH},
        test::collect_string_body,
    };

    use super::*;

    #[derive(serde::Deserialize, serde::Serialize, Clone)]
    struct Gacha<'a> {
        credit_card: &'a str,
    }

    #[test]
    fn extract_lazy() {
        let mut ctx = WebContext::new_test(&());
        let mut ctx = ctx.as_web_ctx();

        let body = serde_json::to_string(&Gacha {
            credit_card: "declined",
        })
        .unwrap();

        ctx.req_mut().headers_mut().insert(CONTENT_TYPE, JSON);
        ctx.req_mut().headers_mut().insert(CONTENT_LENGTH, body.len().into());

        *ctx.body_borrow_mut() = body.into();

        async fn handler(lazy: LazyJson<Gacha<'_>>) -> &'static str {
            let ga = lazy.deserialize().unwrap();
            assert_eq!(ga.credit_card, "declined");
            "bankruptcy"
        }

        let service = handler_service(handler).call(()).now_or_panic().unwrap();

        let body = service.call(ctx).now_or_panic().unwrap().into_body();
        let res = collect_string_body(body).now_or_panic().unwrap();

        assert_eq!(res, "bankruptcy");
    }

    #[test]
    fn service() {
        let res = App::new()
            .at("/", Json(Gacha { credit_card: "mom" }))
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(WebRequest::default())
            .now_or_panic()
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }
}
