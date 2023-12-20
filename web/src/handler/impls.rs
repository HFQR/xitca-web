use core::convert::Infallible;

use crate::{
    body::BodyStream,
    context::WebContext,
    error::Error,
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, WebResponse},
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
    async fn respond_to(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self?.respond_to(ctx).await.map_err(Into::into)
    }
}

impl<'r, C, B, ResB> Responder<WebContext<'r, C, B>> for WebResponse<ResB> {
    type Response = WebResponse<ResB>;
    type Error = Error<C>;

    #[inline]
    async fn respond_to(self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Ok(self)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for Error<C> {
    type Response = WebResponse;
    type Error = Error<C>;

    #[inline]
    async fn respond_to(self, _: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Err(self)
    }
}

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'r, C, B> Responder<WebContext<'r, C, B>> for $type {
            type Response = WebResponse;
            type Error = Infallible;

            async fn respond_to(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                let mut res = ctx.into_response(self);
                res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
                Ok(res)
            }
        }
    };
}

text_utf8!(String);
text_utf8!(&'static str);

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
}
