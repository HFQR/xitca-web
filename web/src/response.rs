pub use xitca_http::{http::response::Builder as WebResponseBuilder, ResponseBody};

use std::future::Future;

use xitca_http::{
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Response},
    util::service::Responder,
};

use super::request::WebRequest;

// TODO: add app state to response type.
pub type WebResponse = Response<ResponseBody>;

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
