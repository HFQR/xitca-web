use crate::{
    body::BodyStream,
    context::WebContext,
    error::Error,
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
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let vec = Vec::from_request(ctx).await?;
        Ok(String::from_utf8(vec).map_err(|e| ExtractError::from(_ParseError::String(e.utf8_error())))?)
    }
}
