use std::{convert::Infallible, future::Future, ops::Deref};

use crate::{handler::Extract, http::Uri, request::WebRequest};

#[derive(Debug)]
pub struct UriRef<'a>(pub &'a Uri);

impl Deref for UriRef<'_> {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, S> Extract<'a, WebRequest<'r, S>> for UriRef<'a>
where
    S: 'static,
{
    type Type<'b> = UriRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    #[inline]
    fn extract(req: &'a WebRequest<'r, S>) -> Self::Future {
        async move { Ok(UriRef(req.req().uri())) }
    }
}
