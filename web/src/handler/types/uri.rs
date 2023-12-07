use std::ops::Deref;

use crate::{
    body::BodyStream,
    context::WebContext,
    handler::{error::ExtractError, FromRequest},
    http::Uri,
};

#[derive(Debug)]
pub struct UriRef<'a>(pub &'a Uri);

impl Deref for UriRef<'_> {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for UriRef<'a>
where
    B: BodyStream,
{
    type Type<'b> = UriRef<'b>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(UriRef(req.req().uri()))
    }
}

#[derive(Debug)]
pub struct UriOwn(pub Uri);

impl Deref for UriOwn {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for UriOwn
where
    B: BodyStream,
{
    type Type<'b> = UriOwn;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(UriOwn(req.req().uri().clone()))
    }
}
