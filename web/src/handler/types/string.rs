use std::convert::Infallible;

use crate::{
    body::BodyStream,
    context::WebContext,
    dev::service::Service,
    error::Error,
    handler::{error::blank_bad_request, FromRequest},
    http::WebResponse,
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

impl<'r, C, B> Service<WebContext<'r, C, B>> for std::string::FromUtf8Error {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Ok(blank_bad_request(ctx))
    }
}
