use crate::{
    body::BodyStream,
    handler::{error::ExtractError, FromRequest},
    request::WebRequest,
};

pub struct Body<B>(pub B);

impl<'r, C, B> FromRequest<WebRequest<'r, C, B>> for Body<B>
where
    B: BodyStream + Default,
{
    type Type<'b> = Body<B>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request<'a>(req: &'a WebRequest<'r, C, B>) -> Result<Self::Type<'a>, Self::Error> {
        Ok(Body(req.take_body_ref()))
    }
}
