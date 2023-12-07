//! type extractor for [Request] and [RequestExt]

use core::ops::Deref;

use crate::{
    body::BodyStream,
    context::WebContext,
    handler::{error::ExtractError, FromRequest},
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
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(RequestRef(req.req()))
    }
}
