//! web context types.

use core::{
    cell::{Ref, RefCell, RefMut},
    mem,
};

use super::{
    body::{RequestBody, ResponseBody},
    handler::FromRequest,
    http::{BorrowReq, BorrowReqMut, IntoResponse, Request, RequestExt, WebRequest, WebResponse},
};

/// web context type focus on stateful and side effect based request data access.
pub struct WebContext<'a, C = (), B = RequestBody> {
    pub(crate) req: &'a mut WebRequest<()>,
    pub(crate) body: &'a mut RefCell<B>,
    pub(crate) ctx: &'a C,
}

impl<'a, C, B> WebContext<'a, C, B> {
    pub(crate) fn new(req: &'a mut WebRequest<()>, body: &'a mut RefCell<B>, ctx: &'a C) -> Self {
        Self { req, body, ctx }
    }

    /// Reborrow Self so the ownership of WebRequest is not lost.
    ///
    /// # Note:
    ///
    /// Reborrow is not pure and receiver of it can mutate Self in any way they like.
    ///
    /// # Example:
    /// ```rust
    /// # use xitca_web::WebContext;
    /// // a function need ownership of request but not return it in output.
    /// fn handler(req: WebContext<'_>) -> Result<(), ()> {
    ///     Err(())
    /// }
    ///
    /// # fn call(mut req: WebContext<'_>) {
    /// // use reborrow to avoid pass request by value.
    /// match handler(req.reborrow()) {
    ///     // still able to access request after handler return.
    ///     Ok(_) => assert_eq!(req.state(), &()),
    ///     Err(_) => assert_eq!(req.state(), &())
    /// }
    /// # }
    /// ```
    #[inline]
    pub fn reborrow(&mut self) -> WebContext<'_, C, B> {
        WebContext {
            req: self.req,
            body: self.body,
            ctx: self.ctx,
        }
    }

    /// extract typed data from WebContext. type must impl [FromRequest] trait.
    /// this is a shortcut method that avoiding explicit import of mentioned trait.
    /// # Examples
    /// ```rust
    /// # use xitca_web::{handler::state::StateRef, WebContext};
    /// async fn extract(ctx: WebContext<'_, usize>) {
    ///     // extract state from context.
    ///     let StateRef(state1) = ctx.extract().await.unwrap();
    ///
    ///     // equivalent of above method with explicit trait import.
    ///     use xitca_web::handler::FromRequest;
    ///     let StateRef(state2) = StateRef::<'_, usize>::from_request(&ctx).await.unwrap();
    ///
    ///     // two extractors will return the same type and data state.
    ///     assert_eq!(state1, state2);
    /// }
    /// ```
    pub async fn extract<'r, T>(&'r self) -> Result<T, T::Error>
    where
        T: FromRequest<'r, Self>,
    {
        T::from_request(self).await
    }

    /// Get an immutable reference of App state
    #[inline]
    pub fn state(&self) -> &C {
        self.ctx
    }

    /// Get an immutable reference of [WebRequest]
    #[inline]
    pub fn req(&self) -> &WebRequest<()> {
        self.req
    }

    /// Get a mutable reference of [WebRequest]
    #[inline]
    pub fn req_mut(&mut self) -> &mut WebRequest<()> {
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

    pub fn take_request(&mut self) -> WebRequest<B>
    where
        B: Default,
    {
        let head = mem::take(self.req_mut());
        let body = self.take_body_mut();
        head.map(|ext| ext.map_body(|_| body))
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

impl<C, B, T> BorrowReq<T> for WebContext<'_, C, B>
where
    Request<RequestExt<()>>: BorrowReq<T>,
{
    fn borrow(&self) -> &T {
        self.req().borrow()
    }
}

impl<C, B, T> BorrowReqMut<T> for WebContext<'_, C, B>
where
    Request<RequestExt<()>>: BorrowReqMut<T>,
{
    fn borrow_mut(&mut self) -> &mut T {
        self.req_mut().borrow_mut()
    }
}

#[cfg(test)]
pub(crate) struct TestWebContext<C> {
    pub(crate) req: Request<RequestExt<()>>,
    pub(crate) body: RefCell<RequestBody>,
    pub(crate) ctx: C,
}

#[cfg(test)]
impl<C> TestWebContext<C> {
    pub(crate) fn as_web_ctx(&mut self) -> WebContext<'_, C> {
        WebContext {
            req: &mut self.req,
            body: &mut self.body,
            ctx: &self.ctx,
        }
    }
}

#[cfg(test)]
impl<C> WebContext<'_, C> {
    pub(crate) fn new_test(ctx: C) -> TestWebContext<C> {
        TestWebContext {
            req: Request::new(RequestExt::default()),
            body: RefCell::new(RequestBody::None),
            ctx,
        }
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use super::*;

    #[test]
    fn extract() {
        use crate::handler::{path::PathRef, state::StateRef};

        let mut ctx = WebContext::new_test(996usize);
        let mut ctx = ctx.as_web_ctx();

        *ctx.req_mut().uri_mut() = crate::http::Uri::from_static("/foo");

        let StateRef(state) = ctx.extract().now_or_panic().ok().unwrap();

        assert_eq!(*state, 996);

        let PathRef(path) = ctx.extract().now_or_panic().ok().unwrap();

        assert_eq!(path, "/foo");
    }
}
