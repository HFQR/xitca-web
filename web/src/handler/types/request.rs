use core::ops::Deref;

use crate::{
    body::BodyStream,
    handler::{error::ExtractError, FromRequest},
    http::{Request, RequestExt},
    request::WebRequest,
};

#[derive(Debug)]
pub struct RequestRef<'a>(pub &'a Request<RequestExt<()>>);

impl Deref for RequestRef<'_> {
    type Target = Request<RequestExt<()>>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'r, C, B> FromRequest<WebRequest<'r, C, B>> for RequestRef<'_>
where
    B: BodyStream,
{
    type Type<'b> = RequestRef<'b>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request<'a>(req: &'a WebRequest<'r, C, B>) -> Result<Self::Type<'a>, Self::Error> {
        Ok(RequestRef(req.req()))
    }
}
