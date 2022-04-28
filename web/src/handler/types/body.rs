use std::{convert::Infallible, future::Future};

use crate::{handler::FromRequest, request::RequestBody, request::WebRequest};

pub struct Body(pub RequestBody);

impl<'a, 'r, 's, S: 's> FromRequest<'a, &'r mut WebRequest<'s, S>> for Body {
    type Type<'b> = Body;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        let extract = Body(std::mem::take(&mut *req.body_borrow_mut()));
        async move { Ok(extract) }
    }
}
