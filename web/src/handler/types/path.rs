use std::ops::Deref;

use crate::{
    body::BodyStream,
    handler::{error::ExtractError, FromRequest},
    request::WebRequest,
};

#[derive(Debug)]
pub struct PathRef<'a>(pub &'a str);

impl Deref for PathRef<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'r, C, B> FromRequest<WebRequest<'r, C, B>> for PathRef<'_>
where
    B: BodyStream,
{
    type Type<'b> = PathRef<'b>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request<'a>(req: &'a WebRequest<'r, C, B>) -> Result<Self::Type<'a>, Self::Error> {
        Ok(PathRef(req.req().uri().path()))
    }
}
