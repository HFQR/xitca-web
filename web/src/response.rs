pub use xitca_http::{http::response::Builder as WebResponseBuilder, ResponseBody};

use std::future::Future;

use xitca_http::{
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Response},
    util::service::Responder,
};

use super::request::WebRequest;

// TODO: add app state to response type.
pub type WebResponse = Response<ResponseBody>;

impl<'a, 'r, 's, S> Responder<'a, &'r mut WebRequest<'s, S>> for String
where
    S: 'static,
{
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: &'a mut &'r mut WebRequest<'s, S>) -> Self::Future {
        async {
            let mut res = req.as_response(self);
            res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
            res
        }
    }
}
