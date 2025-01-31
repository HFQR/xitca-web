//! type extractor, responder and service for text

use core::convert::Infallible;

use crate::{
    body::{BodyStream, ResponseBody},
    context::WebContext,
    error::{Error, forward_blank_bad_request},
    handler::{FromRequest, Responder},
    http::{WebResponse, const_header_value::TEXT_UTF8, header::CONTENT_TYPE},
    service::Service,
};

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for String
where
    B: BodyStream + Default,
{
    type Type<'b> = String;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let vec = Vec::from_request(ctx).await?;
        String::from_utf8(vec).map_err(Error::from_service)
    }
}

forward_blank_bad_request!(std::string::FromUtf8Error);

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'r, C, B> Responder<WebContext<'r, C, B>> for $type {
            type Response = WebResponse;
            type Error = Error;

            #[inline]
            async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
                Text(self).respond(ctx).await
            }

            #[inline]
            fn map(self, res: Self::Response) -> Result<Self::Response, Self::Error> {
                Responder::<WebContext<'r, C, B>>::map(Text(self), res)
            }
        }
    };
}

text_utf8!(&'static str);
text_utf8!(String);
text_utf8!(Box<str>);
text_utf8!(std::borrow::Cow<'static, str>);

/// text responder and service type that would extend [`CONTENT_TYPE`] header with [`TEXT_UTF8`] value to [`WebResponse`].
#[derive(Clone)]
pub struct Text<T>(pub T);

impl<'r, C, B, T> Responder<WebContext<'r, C, B>> for Text<T>
where
    T: Into<ResponseBody>,
{
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let mut res = ctx.into_response(self.0.into());
        res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
        Ok(res)
    }

    fn map(self, res: Self::Response) -> Result<Self::Response, Self::Error> {
        let mut res = res.map(|_| self.0.into());
        res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
        Ok(res)
    }
}

impl<T> Service for Text<T>
where
    T: Clone,
{
    type Response = Self;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        Ok(self.clone())
    }
}

impl<'r, C, B, T> Service<WebContext<'r, C, B>> for Text<T>
where
    T: Into<ResponseBody> + Clone,
{
    type Response = WebResponse;
    type Error = Error;

    #[inline]
    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.clone().respond(ctx).await
    }
}
