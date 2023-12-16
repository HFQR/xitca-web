use core::{convert::Infallible, future::poll_fn, pin::pin};

use crate::{
    body::BodyStream,
    context::WebContext,
    error::Error,
    handler::{ExtractError, FromRequest, Responder},
    http::WebResponse,
};

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Vec<u8>
where
    B: BodyStream + Default,
{
    type Type<'b> = Vec<u8>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let body = ctx.take_body_ref();

        let mut body = pin!(body);

        let mut vec = Vec::new();

        while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            let chunk = chunk.map_err(|e| Error::from_service(ExtractError::Boxed(Box::new(e))))?;

            vec.extend_from_slice(chunk.as_ref());
        }

        Ok(vec)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for Vec<u8> {
    type Response = WebResponse;
    type Error = Infallible;

    async fn respond_to(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        Ok(ctx.into_response(self))
    }
}
