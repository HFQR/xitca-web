//! type extractor for request body stream.

use crate::{
    body::BodyStream,
    handler::{error::ExtractError, FromRequest},
    request::WebRequest,
};

pub struct Body<B>(pub B);

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for Body<B>
where
    B: BodyStream + Default,
{
    type Type<'b> = Body<B>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebRequest<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(Body(req.take_body_ref()))
    }
}
