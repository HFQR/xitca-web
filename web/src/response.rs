pub use xitca_http::{http::response::Builder as WebResponseBuilder, ResponseBody};

use std::future::Future;

use xitca_http::{
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Response},
    util::service::Responder,
};

use super::request::WebRequest;

// TODO: add app state to response type.
pub type WebResponse = Response<ResponseBody>;

impl<'a, 'r, 's, S> Responder<'a, &'r mut WebRequest<'s, S>> for WebResponse {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output> + 'a;

    #[inline]
    fn respond_to(self, _: &'a mut &'r mut WebRequest<'s, S>) -> Self::Future {
        async { self }
    }
}

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'a, 'r, 's, S> Responder<'a, &'r mut WebRequest<'s, S>> for $type
        where
            S: 'static,
        {
            type Output = WebResponse;
            type Future = impl Future<Output = Self::Output> + 'a;

            fn respond_to(self, req: &'a mut &'r mut WebRequest<'s, S>) -> Self::Future {
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
