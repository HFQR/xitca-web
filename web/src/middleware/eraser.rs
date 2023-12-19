//! type eraser middleware.

use core::{cell::RefCell, convert::Infallible, marker::PhantomData};

use std::error;

use crate::{
    body::{BodyStream, BoxBody, RequestBody, ResponseBody},
    bytes::Bytes,
    context::WebContext,
    error::Error,
    http::WebResponse,
    service::{ready::ReadyService, Service},
};

#[doc(hidden)]
mod marker {
    pub struct EraseReqBody;
    pub struct EraseResBody;

    pub struct EraseErr;
}

use marker::*;

/// Type eraser middleware is for erasing "unwanted" complex types produced by service tree
/// and expose well known concrete types `xitca-web` can handle.
///
/// # Example
/// ```rust
/// # use xitca_web::{
/// #   handler::handler_service,
/// #   middleware::{eraser::TypeEraser, limit::Limit},
/// #   App, WebContext
/// #   };
/// // a handler function expect xitca_web::body::RequestBody as body type.
/// async fn handler(_: &WebContext<'_>) -> &'static str {
///     "hello,world!"
/// }
///
/// // a limit middleware that limit request body to max size of 1MB.
/// // this middleware would produce a new type of request body that
/// // handler function don't know of.
/// let limit = Limit::new().set_request_body_max_size(1024 * 1024);
///
/// // an eraser middleware that transform any request body to xitca_web::body::RequestBody.
/// let eraser = TypeEraser::request_body();
///
/// App::new()
///     .at("/", handler_service(handler))
///     // introduce eraser middleware between handler and limit middleware
///     // to bridge the type difference between them.
///     // without it this piece of code would simply refuse to compile.
///     .enclosed(eraser)
///     .enclosed(limit);
/// ```
pub struct TypeEraser<M>(PhantomData<M>);

impl<M> Clone for TypeEraser<M> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<M> TypeEraser<M> {
    fn new() -> Self {
        TypeEraser(PhantomData)
    }
}

impl TypeEraser<EraseReqBody> {
    /// Erase generic request body type. making downstream middlewares observe [RequestBody].
    ///
    /// # Example
    ///
    pub fn request_body() -> Self {
        TypeEraser::new()
    }
}

impl TypeEraser<EraseResBody> {
    /// Erase generic response body type. making downstream middlewares observe [ResponseBody].
    pub fn response_body() -> Self {
        TypeEraser::new()
    }
}

impl TypeEraser<EraseErr> {
    /// Erase generic E type from Service<Error = E>. making downstream middlewares observe [Error].
    pub fn error() -> Self {
        TypeEraser::new()
    }
}

impl<M, S, E> Service<Result<S, E>> for TypeEraser<M> {
    type Response = EraserService<M, S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| EraserService {
            service,
            _erase: PhantomData,
        })
    }
}

pub struct EraserService<M, S> {
    service: S,
    _erase: PhantomData<M>,
}

impl<'r, S, C, ReqB, ResB, Err> Service<WebContext<'r, C, ReqB>> for EraserService<EraseReqBody, S>
where
    S: for<'rs> Service<WebContext<'rs, C>, Response = WebResponse<ResB>, Error = Err>,
    ReqB: BodyStream<Chunk = Bytes> + Default + 'static,
    ResB: BodyStream<Chunk = Bytes> + 'static,
    <ResB as BodyStream>::Error: Send + Sync,
{
    type Response = WebResponse;
    type Error = Err;

    async fn call(&self, mut ctx: WebContext<'r, C, ReqB>) -> Result<Self::Response, Self::Error> {
        let body = ctx.take_body_mut();
        let mut body = RefCell::new(RequestBody::Unknown(BoxBody::new(body)));
        let WebContext { req, ctx, .. } = ctx;
        let res = self.service.call(WebContext::new(req, &mut body, ctx)).await?;
        Ok(res.map(ResponseBody::box_stream))
    }
}

impl<'r, S, C, B, ResB, Err> Service<WebContext<'r, C, B>> for EraserService<EraseResBody, S>
where
    S: for<'rs> Service<WebContext<'rs, C, B>, Response = WebResponse<ResB>, Error = Err>,
    ResB: BodyStream<Chunk = Bytes> + 'static,
    <ResB as BodyStream>::Error: Send + Sync,
{
    type Response = WebResponse;
    type Error = Err;

    #[inline]
    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = self.service.call(ctx).await?;
        Ok(res.map(ResponseBody::box_stream))
    }
}

impl<'r, C, B, S> Service<WebContext<'r, C, B>> for EraserService<EraseErr, S>
where
    S: for<'r2> Service<WebContext<'r, C, B>>,
    S::Error: for<'r2> Service<WebContext<'r2, C>, Response = WebResponse, Error = Infallible>
        + error::Error
        + Send
        + Sync
        + 'static,
{
    type Response = S::Response;
    type Error = Error<C>;

    #[inline]
    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.service.call(ctx).await.map_err(Error::from_service)
    }
}

impl<M, S> ReadyService for EraserService<M, S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.service.ready().await
    }
}

#[cfg(test)]
mod test {
    use xitca_http::{body::Once, Request};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{handler::handler_service, service::ServiceExt, App};

    use super::*;

    async fn handler(_: &WebContext<'_>) -> &'static str {
        "996"
    }

    async fn map_body<S, C, B, Err>(_: &S, _: WebContext<'_, C, B>) -> Result<WebResponse<Once<Bytes>>, Err>
    where
        S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Err>,
    {
        Ok(WebResponse::new(Once::new(Bytes::new())))
    }

    async fn middleware_fn<S, C, B, Err>(s: &S, ctx: WebContext<'_, C, B>) -> Result<WebResponse, Err>
    where
        S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Err>,
    {
        s.call(ctx).await
    }

    #[test]
    fn erase_body() {
        let _ = App::new()
            // map WebResponse to WebResponse<Once<Bytes>> type.
            .at("/", handler_service(handler).enclosed_fn(map_body))
            // earse the body type to make it WebResponse type again.
            .enclosed(TypeEraser::response_body())
            // observe erased body type.
            .enclosed_fn(middleware_fn)
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();
    }
}
