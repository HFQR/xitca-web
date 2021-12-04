use std::{convert::Infallible, future::Future};

use xitca_http::util::service::FromRequest;

use crate::{http::Request, request::WebRequest};

use super::Extract;

impl<'a, 'r, 's, S> FromRequest<'a, &'r mut WebRequest<'s, S>> for Extract<&'a Request<()>>
where
    S: 'static,
{
    type Type<'b> = Extract<&'b Request<()>>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(Extract(req.req())) }
    }
}
