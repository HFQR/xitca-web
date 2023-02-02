use core::{
    future::{poll_fn, Future},
    pin::pin,
};

use crate::{
    body::BodyStream,
    handler::{error::ExtractError, FromRequest, Responder},
    request::WebRequest,
    response::WebResponse,
};

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for Vec<u8>
where
    B: BodyStream + Default,
{
    type Type<'b> = Vec<u8>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        let body = req.take_body_ref();

        async {
            let mut body = pin!(body);

            let mut vec = Vec::new();

            while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                let chunk = chunk.map_err(ExtractError::Body)?;
                vec.extend_from_slice(chunk.as_ref());
            }

            Ok(vec)
        }
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for Vec<u8> {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
        let res = req.into_response(self);
        async { res }
    }
}
