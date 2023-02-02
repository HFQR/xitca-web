use core::{future::Future, ops::Deref};

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

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for RequestRef<'a>
where
    B: BodyStream,
{
    type Type<'b> = RequestRef<'b>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async { Ok(RequestRef(req.req())) }
    }
}
