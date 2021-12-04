use std::{convert::Infallible, future::Future};

use xitca_http::{body::RequestBody, util::service::FromRequest};

use crate::request::WebRequest;

use super::Extract;

impl<'a, 'r, 's, S> FromRequest<'a, &'r mut WebRequest<'s, S>> for Extract<RequestBody>
where
    S: 'static,
{
    type Type<'b> = Extract<RequestBody>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        let extract = Extract(std::mem::take(&mut *req.body_borrow_mut()));
        async move { Ok(extract) }
    }
}
