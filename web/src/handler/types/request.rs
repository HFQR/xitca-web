//! type extractor for [WebRequest] and [RequestExt]

use crate::{
    body::BodyStream,
    context::WebContext,
    error::Error,
    handler::FromRequest,
    http::{RequestExt, WebRequest},
};

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a WebRequest<()>
where
    B: BodyStream,
{
    type Type<'b> = &'b WebRequest<()>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for &'a RequestExt<()>
where
    B: BodyStream,
{
    type Type<'b> = &'b WebRequest<()>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().body())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for WebRequest<()>
where
    B: BodyStream,
{
    type Type<'b> = WebRequest<()>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().clone())
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for RequestExt<()>
where
    B: BodyStream,
{
    type Type<'b> = &'b WebRequest<()>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ctx.req().body().clone())
    }
}
