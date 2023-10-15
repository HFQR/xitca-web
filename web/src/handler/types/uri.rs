use std::ops::Deref;

use crate::{
    body::BodyStream,
    handler::{error::ExtractError, FromRequest},
    http::Uri,
    request::WebRequest,
};

#[derive(Debug)]
pub struct UriRef<'a>(pub &'a Uri);

impl Deref for UriRef<'_> {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'r, C, B> FromRequest<WebRequest<'r, C, B>> for UriRef<'_>
where
    B: BodyStream,
{
    type Type<'b> = UriRef<'b>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request<'a>(req: &'a WebRequest<'r, C, B>) -> Result<Self::Type<'a>, Self::Error> {
        Ok(UriRef(req.req().uri()))
    }
}
