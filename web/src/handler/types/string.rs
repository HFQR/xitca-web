use crate::{
    body::BodyStream,
    context::WebContext,
    handler::{
        error::{ExtractError, _ParseError},
        FromRequest,
    },
};

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for String
where
    B: BodyStream + Default,
{
    type Type<'b> = String;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let vec = Vec::from_request(req).await?;
        Ok(String::from_utf8(vec).map_err(|e| _ParseError::String(e.utf8_error()))?)
    }
}
