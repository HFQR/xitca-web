//! type extractor for request body stream.

use crate::{body::BodyStream, context::WebContext, error::Error, handler::FromRequest};

pub struct Body<B>(pub B);

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Body<B>
where
    B: BodyStream + Default,
{
    type Type<'b> = Body<B>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(Body(ctx.take_body_ref()))
    }
}
