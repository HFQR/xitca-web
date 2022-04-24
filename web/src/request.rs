use std::{
    cell::{Ref, RefCell, RefMut},
    mem,
};

use xitca_http::{
    http::IntoResponse,
    request::{BorrowReq, Request},
    RequestBody, ResponseBody,
};

use super::response::WebResponse;

pub struct WebRequest<'a, D = ()> {
    pub(crate) req: Request<()>,
    pub(crate) body: RefCell<RequestBody>,
    pub(crate) state: &'a D,
}

impl<'a, D> WebRequest<'a, D> {
    #[doc(hidden)]
    pub fn new(req: &mut Request<RequestBody>, state: &'a D) -> Self {
        let (req, body) = std::mem::take(req).replace_body(());
        Self {
            req,
            body: RefCell::new(body),
            state,
        }
    }

    #[cfg(test)]
    pub fn with_state(state: &'a D) -> Self {
        Self::new(&mut Request::new(RequestBody::None), state)
    }

    /// Get an immutable reference of [Request]
    #[inline]
    pub fn req(&self) -> &Request<()> {
        &self.req
    }

    /// Get a mutable reference of [Request]
    #[inline]
    pub fn req_mut(&mut self) -> &mut Request<()> {
        &mut self.req
    }

    /// Get a immutable reference of [RequestBody]
    #[inline]
    pub fn body(&self) -> Ref<'_, RequestBody> {
        self.body.borrow()
    }

    /// Get a mutable reference of [RequestBody]
    #[inline]
    pub fn body_borrow_mut(&self) -> RefMut<'_, RequestBody> {
        self.body.borrow_mut()
    }

    /// Get a mutable reference of [RequestBody]
    /// This API takes &mut WebRequest so it bypass runtime borrow checker
    /// and therefore has zero runtime overhead.
    #[inline]
    pub fn body_get_mut(&mut self) -> &mut RequestBody {
        self.body.get_mut()
    }

    pub fn take_request(&mut self) -> Request<RequestBody> {
        let head = mem::take(self.req_mut());
        let body = mem::take(self.body_get_mut());
        head.map_body(|_| body)
    }

    /// Get an immutable reference of App state
    #[inline]
    pub fn state(&self) -> &D {
        self.state
    }

    /// Transform self to a WebResponse with given body type.
    ///
    /// The heap allocation of request would be re-used.
    #[inline]
    pub fn into_response<B: Into<ResponseBody>>(self, body: B) -> WebResponse {
        self.req.into_response(body.into())
    }

    /// Transform &mut self to a WebResponse with given body type.
    ///
    /// The heap allocation of request would be re-used.
    #[inline]
    pub fn as_response<B: Into<ResponseBody>>(&mut self, body: B) -> WebResponse {
        self.req.as_response(body.into())
    }
}

impl<S, T> BorrowReq<T> for &mut WebRequest<'_, S>
where
    Request<()>: BorrowReq<T>,
{
    fn borrow(&self) -> &T {
        self.req().borrow()
    }
}
