use std::{convert::Infallible, future::Future, ops::Deref};

use xitca_http::util::service::FromRequest;

use crate::{http::Uri, request::WebRequest};

#[derive(Debug)]
pub struct UriRef<'a>(pub &'a Uri);

impl Deref for UriRef<'_> {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, 's, S> FromRequest<'a, &'r mut WebRequest<'s, S>> for UriRef<'a>
where
    S: 'static,
{
    type Type<'b> = UriRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(UriRef(req.req().uri())) }
    }
}
