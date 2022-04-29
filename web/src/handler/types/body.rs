use std::{convert::Infallible, future::Future};

use crate::{handler::FromRequest, request::RequestBody, request::WebRequest};

pub struct Body(pub RequestBody);

impl<'a, 'r, S: 'r> FromRequest<'a, WebRequest<'r, S>> for Body {
    type Type<'b> = Body;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, S>) -> Self::Future {
        let extract = Body(std::mem::take(&mut *req.body_borrow_mut()));
        async move { Ok(extract) }
    }
}
