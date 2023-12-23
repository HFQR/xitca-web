use core::convert::Infallible;

use crate::{
    body::BodyStream,
    bytes::Bytes,
    context::WebContext,
    error::Error,
    http::{
        const_header_value::TEXT_UTF8, header::CONTENT_TYPE, HeaderMap, RequestExt, StatusCode, WebRequest, WebResponse,
    },
};

use super::{FromRequest, Responder};

impl<'a, 'r, C, B, T, E> FromRequest<'a, WebContext<'r, C, B>> for Result<T, E>
where
    B: BodyStream,
    T: for<'a2, 'r2> FromRequest<'a2, WebContext<'r2, C, B>, Error = E>,
{
    type Type<'b> = Result<T, E>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(T::from_request(ctx).await)
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for Option<T>
where
    B: BodyStream,
    T: for<'a2, 'r2> FromRequest<'a2, WebContext<'r2, C, B>>,
{
    type Type<'b> = Option<T>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(T::from_request(ctx).await.ok())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a WebContext<'a, C, B>
where
    C: 'static,
    B: BodyStream + 'static,
{
    type Type<'b> = &'b WebContext<'b, C, B>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx)
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a WebRequest<()>
where
    B: BodyStream,
{
    type Type<'b> = &'b WebRequest<()>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a RequestExt<()>
where
    B: BodyStream,
{
    type Type<'b> = &'b RequestExt<()>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().body())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for WebRequest<()>
where
    B: BodyStream,
{
    type Type<'b> = WebRequest<()>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().clone())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for RequestExt<()>
where
    B: BodyStream,
{
    type Type<'b> = RequestExt<()>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().body().clone())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for ()
where
    B: BodyStream,
{
    type Type<'b> = ();
    type Error = Error<C>;

    #[inline]
    async fn from_request(_: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(())
    }
}

impl<'r, C, B, T, Res, Err, E> Responder<WebContext<'r, C, B>> for Result<T, E>
where
    T: for<'r2> Responder<WebContext<'r2, C, B>, Response = Res, Error = Err>,
    Error<C>: From<E> + From<Err>,
{
    type Response = Res;
    type Error = Error<C>;

    #[inline]
    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self?.respond(ctx).await.map_err(Into::into)
    }
}

impl<'r, C, B, ResB> Responder<WebContext<'r, C, B>> for WebResponse<ResB> {
    type Response = WebResponse<ResB>;
    type Error = Error<C>;

    #[inline]
    async fn respond(self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Ok(self)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for Error<C> {
    type Response = WebResponse;
    type Error = Error<C>;

    #[inline]
    async fn respond(self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Err(self)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for StatusCode {
    type Response = WebResponse;
    type Error = Infallible;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(Bytes::new());
        <Self as Responder<WebContext<'r, C, B>>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        *res.status_mut() = self;
        Ok(res)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for HeaderMap {
    type Response = WebResponse;
    type Error = Infallible;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(Bytes::new());
        <Self as Responder<WebContext<'r, C, B>>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        res.headers_mut().extend(self);
        Ok(res)
    }
}

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'r, C, B> Responder<WebContext<'r, C, B>> for $type {
            type Response = WebResponse;
            type Error = Infallible;

            async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                let mut res = ctx.into_response(self);
                res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
                Ok(res)
            }

            fn map(self, res: Self::Response) -> Result<Self::Response, Self::Error> {
                let mut res = res.map(|_| self.into());
                res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
                Ok(res)
            }
        }
    };
}

text_utf8!(&'static str);
text_utf8!(String);
text_utf8!(Box<str>);
text_utf8!(std::borrow::Cow<'static, str>);

// shared error impl for serde enabled features: json, urlencoded, etc.
#[cfg(feature = "serde")]
impl<'r, C, B> crate::service::Service<WebContext<'r, C, B>> for serde::de::value::Error {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        crate::error::BadRequest.call(ctx).await
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::http::header::{HeaderValue, COOKIE};

    use super::*;

    #[test]
    fn extract_default_impls() {
        let mut req = WebContext::new_test(());
        let req = req.as_web_ctx();

        Option::<()>::from_request(&req).now_or_panic().unwrap().unwrap();

        Result::<(), Error<()>>::from_request(&req)
            .now_or_panic()
            .unwrap()
            .unwrap();

        <&WebContext<'_>>::from_request(&req).now_or_panic().unwrap();

        <()>::from_request(&req).now_or_panic().unwrap();
    }

    #[test]
    fn respond_chain() {
        let mut req = WebContext::new_test(());
        let mut req = req.as_web_ctx();

        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("none"));

        let check_res = |res: WebResponse| {
            assert_eq!(res.status(), StatusCode::ACCEPTED);
            assert_eq!(res.headers().get(COOKIE).unwrap().to_str().unwrap(), "none");
            assert_eq!(
                res.headers().get(CONTENT_TYPE).unwrap().to_str().unwrap(),
                "text/plain; charset=utf-8"
            );
        };

        let res = (StatusCode::ACCEPTED, headers.clone(), "hello,world!")
            .respond(req.reborrow())
            .now_or_panic()
            .unwrap();
        check_res(res);

        let res = ("hello,world!", StatusCode::ACCEPTED, headers.clone())
            .respond(req.reborrow())
            .now_or_panic()
            .unwrap();
        check_res(res);

        let res = (headers, "hello,world!", StatusCode::ACCEPTED)
            .respond(req)
            .now_or_panic()
            .unwrap();
        check_res(res);
    }
}
