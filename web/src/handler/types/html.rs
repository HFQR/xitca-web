use std::{fmt, future::Future};

use xitca_http::body::ResponseBody;

use crate::{
    handler::Responder,
    http::{const_header_value::TEXT_HTML_UTF8, header::CONTENT_TYPE},
    request::WebRequest,
    response::WebResponse,
};

pub struct Html<T>(pub T);

impl<T> fmt::Debug for Html<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Html").field("value", &self.0).finish()
    }
}

impl<'r, S, T> Responder<WebRequest<'r, S>> for Html<T>
where
    T: Into<ResponseBody>,
{
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    #[inline]
    fn respond_to(self, req: WebRequest<'r, S>) -> Self::Future {
        let mut res = req.into_response(self.0);
        res.headers_mut().insert(CONTENT_TYPE, TEXT_HTML_UTF8);
        async { res }
    }
}
