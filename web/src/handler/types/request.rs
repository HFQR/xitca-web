//! type extractor for [Request] and [RequestExt]

use core::ops::Deref;

use crate::{
    body::BodyStream,
    context::WebContext,
    error::Error,
    handler::FromRequest,
    http::{Request, RequestExt},
};

#[derive(Debug)]
pub struct RequestRef<'a>(pub &'a Request<RequestExt<()>>);

impl Deref for RequestRef<'_> {
    type Target = Request<RequestExt<()>>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for RequestRef<'a>
where
    B: BodyStream,
{
    type Type<'b> = RequestRef<'b>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(RequestRef(ctx.req()))
    }
}
