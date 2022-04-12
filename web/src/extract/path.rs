use std::{convert::Infallible, future::Future, ops::Deref};

use xitca_http::util::service::FromRequest;

use crate::request::WebRequest;

#[derive(Debug)]
pub struct PathRef<'a>(pub &'a str);

impl Deref for PathRef<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, 's, S: 's> FromRequest<'a, &'r mut WebRequest<'s, S>> for PathRef<'a> {
    type Type<'b> = PathRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(PathRef(req.req().uri().path())) }
    }
}
