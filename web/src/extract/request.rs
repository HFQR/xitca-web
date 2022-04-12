use std::{convert::Infallible, future::Future, ops::Deref};

use xitca_http::util::service::FromRequest;

use crate::{http::Request, request::WebRequest};

#[derive(Debug)]
pub struct RequestRef<'a>(pub &'a Request<()>);

impl Deref for RequestRef<'_> {
    type Target = Request<()>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, 's, S: 's> FromRequest<'a, &'r mut WebRequest<'s, S>> for RequestRef<'a> {
    type Type<'b> = RequestRef<'b>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(RequestRef(req.req())) }
    }
}
