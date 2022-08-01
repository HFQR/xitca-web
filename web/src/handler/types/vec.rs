use std::future::{poll_fn, Future};

use xitca_unsafe_collection::pin;

use crate::{
    handler::{error::ExtractError, FromRequest, Responder},
    request::WebRequest,
    response::WebResponse,
    stream::WebStream,
};

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for Vec<u8>
where
    B: WebStream,
{
    type Type<'b> = Vec<u8>;
    type Error = ExtractError;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        let body = req.take_body_ref();

        async {
            pin!(body);

            let mut vec = Vec::new();

            while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                let chunk = chunk.map_err(|_| ExtractError::UnexpectedEof)?;
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
