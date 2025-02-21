use core::net::SocketAddr;

use crate::{
    body::ResponseBody,
    context::WebContext,
    error::{Error, ErrorStatus},
    http::{Method, RequestExt, StatusCode, WebRequest, WebResponse},
};

use super::{FromRequest, Responder};

impl<'a, 'r, C, B, T, E> FromRequest<'a, WebContext<'r, C, B>> for Result<T, E>
where
    T: FromRequest<'a, WebContext<'r, C, B>, Error = E>,
{
    type Type<'b> = Result<T::Type<'b>, T::Error>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(T::from_request(ctx).await)
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for Option<T>
where
    T: FromRequest<'a, WebContext<'r, C, B>>,
{
    type Type<'b> = Option<T::Type<'b>>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(T::from_request(ctx).await.ok())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a WebContext<'a, C, B>
where
    C: 'static,
    B: 'static,
{
    type Type<'b> = &'b WebContext<'b, C, B>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx)
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a WebRequest<()> {
    type Type<'b> = &'b WebRequest<()>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for WebRequest<()> {
    type Type<'b> = WebRequest<()>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().clone())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a RequestExt<()> {
    type Type<'b> = &'b RequestExt<()>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().body())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for RequestExt<()> {
    type Type<'b> = RequestExt<()>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().body().clone())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a SocketAddr {
    type Type<'b> = &'b SocketAddr;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().body().socket_addr())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for SocketAddr {
    type Type<'b> = SocketAddr;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(*ctx.req().body().socket_addr())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a Method {
    type Type<'b> = &'b Method;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().method())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Method {
    type Type<'b> = Method;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().method().clone())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for () {
    type Type<'b> = ();
    type Error = Error;

    #[inline]
    async fn from_request(_: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(())
    }
}

impl<'r, C, B, T, Res, Err, E> Responder<WebContext<'r, C, B>> for Result<T, E>
where
    T: for<'r2> Responder<WebContext<'r2, C, B>, Response = Res, Error = Err>,
    Error: From<E> + From<Err>,
{
    type Response = Res;
    type Error = Error;

    #[inline]
    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self?.respond(ctx).await.map_err(Into::into)
    }
}

impl<'r, C, B, ResB> Responder<WebContext<'r, C, B>> for WebResponse<ResB> {
    type Response = WebResponse<ResB>;
    type Error = Error;

    #[inline]
    async fn respond(self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Ok(self)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for Error {
    type Response = WebResponse;
    type Error = Error;

    #[inline]
    async fn respond(self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Err(self)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for ErrorStatus {
    type Response = WebResponse;
    type Error = Error;

    #[inline]
    async fn respond(self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Err(Error::from(self))
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for StatusCode {
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(ResponseBody::empty());
        Responder::<WebContext<'r, C, B>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        *res.status_mut() = self;
        Ok(res)
    }
}

// shared error impl for serde enabled features: json, urlencoded, etc.
#[cfg(feature = "serde")]
const _: () = {
    crate::error::error_from_service!(serde::de::value::Error);
    crate::error::forward_blank_bad_request!(serde::de::value::Error);
};

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::http::header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue};

    use super::*;

    #[test]
    fn extract_default_impls() {
        let mut req = WebContext::new_test(());
        let req = req.as_web_ctx();

        Option::<()>::from_request(&req).now_or_panic().unwrap().unwrap();

        Result::<(), Error>::from_request(&req).now_or_panic().unwrap().unwrap();

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
