use std::{convert::Infallible, fmt::Debug, future::Future};

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
        impl<'r, 's, S> Responder<&'r mut WebRequest<'s, S>> for $type {
            type Output = WebResponse;
            type Future<'a> = impl Future<Output = Self::Output> where &'r mut WebRequest<'s, S>: 'a;

            fn respond_to<'a>(self, req: &'a mut &'r mut WebRequest<'s, S>) -> Self::Future<'a> {
                async move {
                    let mut res = req.as_response(self);
                    res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
                    res
                }
            }
        }
    };
}

text_utf8!(String);
text_utf8!(&'static str);

impl<'r, 's, S, T, E> Responder<&'r mut WebRequest<'s, S>> for Result<T, E>
where
    T: Responder<&'r mut WebRequest<'s, S>>,
    E: Debug,
{
    type Output = Result<T::Output, E>;
    type Future<'a> = impl Future<Output = Self::Output> where &'r mut WebRequest<'s, S>: 'a;

    #[inline]
    fn respond_to<'a>(self, req: &'a mut &'r mut WebRequest<'s, S>) -> Self::Future<'a> {
        async move { Ok(self?.respond_to(req).await) }
    }
}
