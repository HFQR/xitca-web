use crate::{
    body::BodyStream,
    context::WebContext,
    error::{forward_blank_bad_request, Error},
    handler::FromRequest,
    http::WebResponse,
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
        String::from_utf8(vec).map_err(Error::from_service)
    }
}

forward_blank_bad_request!(std::string::FromUtf8Error);
