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

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for UriRef<'a>
where
    B: BodyStream,
{
    type Type<'b> = UriRef<'b>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebRequest<'r, C, B>) -> Result<Self, Self::Error> {
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

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for UriOwn
where
    B: BodyStream,
{
    type Type<'b> = UriOwn;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebRequest<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(UriOwn(req.req().uri().clone()))
    }
}
