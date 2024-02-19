//! type extractor for request uri path

use core::ops::Deref;

use crate::{context::WebContext, error::Error, handler::FromRequest};

#[derive(Debug)]
pub struct PathRef<'a>(pub &'a str);

impl Deref for PathRef<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for PathRef<'a> {
    type Type<'b> = PathRef<'b>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(PathRef(ctx.req().uri().path()))
    }
}

#[derive(Debug)]
pub struct PathOwn(pub String);

impl Deref for PathOwn {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_str()
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for PathOwn {
    type Type<'b> = PathOwn;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(PathOwn(ctx.req().uri().path().to_string()))
    }
}
