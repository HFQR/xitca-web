use core::ops::Deref;

use crate::{context::WebContext, error::Error, handler::FromRequest, http::Uri};

#[derive(Debug)]
pub struct UriRef<'a>(pub &'a Uri);

impl Deref for UriRef<'_> {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for UriRef<'a> {
    type Type<'b> = UriRef<'b>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(UriRef(ctx.req().uri()))
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a Uri {
    type Type<'b> = &'b Uri;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().uri())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for Uri {
    type Type<'b> = Uri;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().uri().clone())
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

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for UriOwn {
    type Type<'b> = UriOwn;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(UriOwn(ctx.req().uri().clone()))
    }
}
