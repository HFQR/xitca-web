pub use xitca_http::body::RequestBody;

use std::{
    cell::{Ref, RefCell, RefMut},
    mem,
};

use xitca_http::{
    http::IntoResponse,
    request::{BorrowReq, Request},
    ResponseBody,
};

use super::response::WebResponse;

pub struct WebRequest<'a, C = (), B = RequestBody> {
    pub(crate) req: &'a mut Request<()>,
    pub(crate) body: &'a mut RefCell<B>,
    pub(crate) ctx: &'a C,
}

impl<'a, C, B> WebRequest<'a, C, B> {
    pub(crate) fn new(req: &'a mut Request<()>, body: &'a mut RefCell<B>, ctx: &'a C) -> Self {
        Self { req, body, ctx }
    }

    /// Construct an owned Self from &mut Self.
    ///
    /// This is possible because all fields of `WebRequest` are Copy.
    ///
    /// # Example:
    /// ```rust
    /// # use xitca_web::request::WebRequest;
    /// // a function need ownership of request but not return it in output.
    /// fn handler(req: WebRequest<'_>) -> Result<(), ()> {
    ///     Err(())
    /// }
    ///
    /// # fn call(mut req: WebRequest<'_>) {
    /// // use reborrow to avoid pass request by value.
    /// match handler(req.reborrow()) {
    ///     // still able to access request after handler return.
    ///     Ok(_) => assert_eq!(req.state(), &()),
    ///     Err(_) => assert_eq!(req.state(), &())
    /// }
    /// # }
    /// ```
    #[inline]
    pub fn reborrow(&mut self) -> WebRequest<'_, C, B> {
        WebRequest {
            req: self.req,
            body: self.body,
            ctx: self.ctx,
        }
    }

    /// Get an immutable reference of App state
    #[inline]
    pub fn state(&self) -> &C {
        self.ctx
    }

    /// Get an immutable reference of [Request]
    #[inline]
    pub fn req(&self) -> &Request<()> {
        self.req
    }

    /// Get a mutable reference of [Request]
    #[inline]
    pub fn req_mut(&mut self) -> &mut Request<()> {
        self.req
    }

    /// Get a immutable reference of [RequestBody]
    #[inline]
    pub fn body(&self) -> Ref<'_, B> {
        self.body.borrow()
    }

    /// Get a mutable reference of [RequestBody]
    #[inline]
    pub fn body_borrow_mut(&self) -> RefMut<'_, B> {
        self.body.borrow_mut()
    }

    /// Get a mutable reference of [RequestBody]
    /// This API takes &mut WebRequest so it bypass runtime borrow checker
    /// and therefore has zero runtime overhead.
    #[inline]
    pub fn body_get_mut(&mut self) -> &mut B {
        self.body.get_mut()
    }

    pub fn take_request(&mut self) -> Request<B>
    where
        B: Default,
    {
        let head = mem::take(self.req_mut());
        let body = self.take_body_mut();
        head.map_body(|_| body)
    }

    /// Transform self to a WebResponse with given body type.
    ///
    /// The heap allocation of request would be re-used.
    #[inline]
    pub fn into_response<ResB: Into<ResponseBody>>(self, body: ResB) -> WebResponse {
        self.req.as_response(body.into())
    }

    /// Transform &mut self to a WebResponse with given body type.
    ///
    /// The heap allocation of request would be re-used.
    #[inline]
    pub fn as_response<ResB: Into<ResponseBody>>(&mut self, body: ResB) -> WebResponse {
        self.req.as_response(body.into())
    }

    pub(crate) fn take_body_ref(&self) -> B
    where
        B: Default,
    {
        mem::take(&mut *self.body_borrow_mut())
    }

    pub(crate) fn take_body_mut(&mut self) -> B
    where
        B: Default,
    {
        mem::take(self.body_get_mut())
    }
}

#[cfg(test)]
impl<C> WebRequest<'_, C> {
    pub(crate) fn new_test(ctx: C) -> TestRequest<C> {
        TestRequest {
            req: Request::new(()),
            body: RefCell::new(RequestBody::None),
            ctx,
        }
    }
}

impl<C, B, T> BorrowReq<T> for WebRequest<'_, C, B>
where
    Request<()>: BorrowReq<T>,
{
    fn borrow(&self) -> &T {
        self.req().borrow()
    }
}

#[cfg(test)]
pub(crate) struct TestRequest<C> {
    pub(crate) req: Request<()>,
    pub(crate) body: RefCell<RequestBody>,
    pub(crate) ctx: C,
}

#[cfg(test)]
impl<C> TestRequest<C> {
    pub(crate) fn as_web_req(&mut self) -> WebRequest<'_, C> {
        WebRequest {
            req: &mut self.req,
            body: &mut self.body,
            ctx: &self.ctx,
        }
    }
}
