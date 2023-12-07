use core::{future::poll_fn, pin::pin};

use crate::{
    body::BodyStream,
    context::WebContext,
    handler::{error::ExtractError, FromRequest, Responder},
    http::WebResponse,
};

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Vec<u8>
where
    B: BodyStream + Default,
{
    type Type<'b> = Vec<u8>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let body = req.take_body_ref();

        let mut body = pin!(body);

        let mut vec = Vec::new();

        while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            let chunk = chunk.map_err(ExtractError::Body)?;
            vec.extend_from_slice(chunk.as_ref());
        }

        Ok(vec)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for Vec<u8> {
    type Output = WebResponse;

    async fn respond_to(self, req: WebContext<'r, C, B>) -> Self::Output {
        req.into_response(self)
    }
}
