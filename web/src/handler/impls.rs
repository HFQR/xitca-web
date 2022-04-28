use std::{convert::Infallible, future::Future};

use xitca_http::http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE};

use crate::{request::WebRequest, response::WebResponse};

use super::{FromRequest, Responder};

impl<'a, 'r, 's, S> FromRequest<'a, &'r mut WebRequest<'s, S>> for &'a WebRequest<'a, S>
where
    S: 'static,
{
    type Type<'b> = &'b WebRequest<'b, S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
        async move { Ok(&**req) }
    }
}

impl<'r, 's, S> Responder<&'r mut WebRequest<'s, S>> for WebResponse {
    type Output = WebResponse;
    type Future<'a> = impl Future<Output = Self::Output> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn respond_to<'a>(self, _: &'a mut &'r mut WebRequest<'s, S>) -> Self::Future<'a> {
        async { self }
    }
}

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'r, 's, S: 's> Responder<&'r mut WebRequest<'s, S>> for $type {
            type Output = WebResponse;
            type Future<'a> = impl Future<Output = Self::Output> where &'r mut WebRequest<'s, S>: 'a;

            fn respond_to<'a>(self, req: &'a mut &'r mut WebRequest<'s, S>) -> Self::Future<'a> {
                let mut res = req.as_response(self);
                res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
                async { res }
            }
        }
    };
}

text_utf8!(String);
text_utf8!(&'static str);
