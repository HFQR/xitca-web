use std::{convert::Infallible, future::Future, ops::Deref};

use crate::{handler::FromRequest, http::Uri, request::WebRequest};

#[derive(Debug)]
pub struct UriRef<'a>(pub &'a Uri);

impl Deref for UriRef<'_> {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for UriRef<'a> {
    type Type<'b> = UriRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async move { Ok(UriRef(req.req().uri())) }
    }
}
