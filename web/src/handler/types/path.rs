use std::{convert::Infallible, future::Future, ops::Deref};

use crate::{handler::FromRequest, request::WebRequest};

#[derive(Debug)]
pub struct PathRef<'a>(pub &'a str);

impl Deref for PathRef<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, S: 'r> FromRequest<'a, WebRequest<'r, S>> for PathRef<'a> {
    type Type<'b> = PathRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, S>) -> Self::Future {
        async move { Ok(PathRef(req.req().uri().path())) }
    }
}
