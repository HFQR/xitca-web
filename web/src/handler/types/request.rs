use std::{convert::Infallible, future::Future, ops::Deref};

use crate::{handler::FromRequest, http::Request, request::WebRequest};

#[derive(Debug)]
pub struct RequestRef<'a>(pub &'a Request<()>);

impl Deref for RequestRef<'_> {
    type Target = Request<()>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for RequestRef<'a> {
    type Type<'b> = RequestRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async { Ok(RequestRef(req.req())) }
    }
}
