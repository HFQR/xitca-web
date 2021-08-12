use std::{
    cell::{Ref, RefCell, RefMut},
    mem,
};

use xitca_http::{
    http::{request::Parts, Request},
    RequestBody, ResponseBody,
};

use super::response::WebResponse;

pub struct WebRequest<'a, D> {
    pub(crate) http: RefCell<Request<RequestBody>>,
    pub(crate) state: &'a D,
}

impl<'a, D> WebRequest<'a, D> {
    #[doc(hidden)]
    pub fn new(http: Request<RequestBody>, state: &'a D) -> Self {
        Self {
            http: RefCell::new(http),
            state,
        }
    }

    #[cfg(test)]
    pub fn with_state(state: &'a D) -> Self {
        Self {
            http: RefCell::new(Request::default()),
            state,
        }
    }

    /// Get an immutable reference of [Request](xitca_http::http::Request)
    #[inline]
    pub fn request_ref(&self) -> Ref<'_, Request<RequestBody>> {
        self.http.borrow()
    }

    /// Get a mutable reference of [Request](xitca_http::http::Request)
    #[inline]
    pub fn request_ref_mut(&self) -> RefMut<'_, Request<RequestBody>> {
        self.http.borrow_mut()
    }

    /// Get a mutable reference of [Request](xitca_http::http::Request)
    /// This API takes &mut WebRequest so it bypass runtime borrow checker
    /// and therefore has zero runtime overhead.
    #[inline]
    pub fn request_mut(&mut self) -> &mut Request<RequestBody> {
        self.http.get_mut()
    }

    /// Get an immutable reference of App state
    #[inline]
    pub fn state(&self) -> &D {
        self.state
    }

    /// Transform self to a WebResponse with given body type.
    ///
    /// The heap allocation of request would be re-used.
    #[inline(always)]
    pub fn into_response<B: Into<ResponseBody>>(mut self, body: B) -> WebResponse {
        self.as_response(body)
    }

    /// Transform &mut self to a WebResponse with given body type.
    ///
    /// The heap allocation of request would be re-used.
    pub fn as_response<B: Into<ResponseBody>>(&mut self, body: B) -> WebResponse {
        let Parts {
            mut headers,
            extensions,
            ..
        } = mem::take(self.request_mut()).into_parts().0;

        headers.clear();

        let mut res = WebResponse::new(body.into());
        *res.headers_mut() = headers;
        *res.extensions_mut() = extensions;

        res
    }
}
