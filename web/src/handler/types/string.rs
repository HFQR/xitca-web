use crate::{
    body::BodyStream,
    context::WebContext,
    error::{forward_blank_bad_request, Error},
    handler::{FromRequest, Responder},
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, WebResponse},
};

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for String
where
    B: BodyStream + Default,
{
    type Type<'b> = String;
    type Error = Error<C>;

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
            type Error = Error<C>;

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
