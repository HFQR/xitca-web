use std::{convert::Infallible, future::Future, ops::Deref};

use crate::{handler::Extract, http::Request, request::WebRequest};

#[derive(Debug)]
pub struct RequestRef<'a>(pub &'a Request<()>);

impl Deref for RequestRef<'_> {
    type Target = Request<()>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, S: 'r> Extract<'a, WebRequest<'r, S>> for RequestRef<'a> {
    type Type<'b> = RequestRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    #[inline]
    fn extract(req: &'a WebRequest<'r, S>) -> Self::Future {
        async { Ok(RequestRef(req.req())) }
    }
}
