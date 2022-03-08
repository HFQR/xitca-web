/// Default implement for handler module. optional enabled.
use std::{convert::Infallible, future::Future};

use super::handler::{FromRequest, Responder};

use crate::{
    body::{RequestBody, ResponseBody},
    bytes::Bytes,
    http::{const_header_value, header, IntoResponse, Response},
    Request,
};

impl<'a, Req> FromRequest<'a, Req> for () {
    type Type<'b> = ();
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>>;

    #[inline(always)]
    fn from_request(_: &'a Req) -> Self::Future {
        async { Ok(()) }
    }
}

impl<'a> Responder<'a, Request<RequestBody>> for () {
    type Output = Response<ResponseBody>;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: &'a mut Request<RequestBody>) -> Self::Future {
        async move { req.as_response(Bytes::new()) }
    }
}

impl<'a> Responder<'a, Request<RequestBody>> for &'_ str {
    type Output = Response<ResponseBody>;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: &'a mut Request<RequestBody>) -> Self::Future {
        async move {
            let mut res = req.as_response(self);
            res.headers_mut()
                .insert(header::CONTENT_TYPE, const_header_value::TEXT_UTF8);
            res
        }
    }
}
