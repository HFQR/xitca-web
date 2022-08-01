use std::future::Future;

use crate::{
    handler::{error::ExtractError, FromRequest},
    request::WebRequest,
};

pub struct Body<B>(pub B);

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for Body<B>
where
    B: Default,
{
    type Type<'b> = Body<B>;
    type Error = ExtractError;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        let extract = Body(req.take_body_ref());
        async { Ok(extract) }
    }
}
