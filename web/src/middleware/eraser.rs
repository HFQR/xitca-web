//! type eraser middleware.

use core::marker::PhantomData;

use crate::service::Service;

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
/// #   middleware::{eraser::TypeEraser, limit::Limit, Group},
/// #   service::ServiceExt,
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
///     // to resolve the type difference between them.
///     // without it this piece of code would simply refuse to compile.
///     .enclosed(eraser.clone())
///     .enclosed(limit.clone());
///
/// // group middleware is suggested way of handling of use case like this.
/// let group = Group::new()
///     .enclosed(eraser)
///     .enclosed(limit);
///
/// // it's also suggested to group multiple type mutation middlewares together and apply
/// // eraser on them once if possible. the reason being TypeErase has a cost and by
/// // grouping you only pay the cost once.
///
/// App::new()
///     .at("/", handler_service(handler))
///     .enclosed(group);
/// ```
pub struct TypeEraser<M>(PhantomData<M>);

impl<M> Clone for TypeEraser<M> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl TypeEraser<EraseReqBody> {
    /// Erase generic request body type. making downstream middlewares observe [`RequestBody`].
    ///
    /// [`RequestBody`]: crate::body::RequestBody
    pub const fn request_body() -> Self {
        TypeEraser(PhantomData)
    }
}

impl TypeEraser<EraseResBody> {
    /// Erase generic response body type. making downstream middlewares observe [`ResponseBody`].
    ///
    /// [`ResponseBody`]: crate::body::ResponseBody
    pub const fn response_body() -> Self {
        TypeEraser(PhantomData)
    }
}

impl TypeEraser<EraseErr> {
    /// Erase generic E type from Service<Error = E>. making downstream middlewares observe [`Error`].
    ///
    /// [`Error`]: crate::error::Error
    pub const fn error() -> Self {
        TypeEraser(PhantomData)
    }
}

impl<M, S, E> Service<Result<S, E>> for TypeEraser<M> {
    type Response = service::EraserService<M, S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| service::EraserService {
            service,
            _erase: PhantomData,
        })
    }
}

mod service {
    use core::cell::RefCell;

    use crate::{
        WebContext,
        body::{BodyStream, BoxBody},
        body::{RequestBody, ResponseBody},
        bytes::Bytes,
        error::Error,
        http::WebResponse,
        service::ready::ReadyService,
    };

    use super::*;

    pub struct EraserService<M, S> {
        pub(super) service: S,
        pub(super) _erase: PhantomData<M>,
    }

    impl<'r, S, C, ReqB, ResB, Err> Service<WebContext<'r, C, ReqB>> for EraserService<EraseReqBody, S>
    where
        S: for<'rs> Service<WebContext<'rs, C>, Response = WebResponse<ResB>, Error = Err>,
        ReqB: BodyStream<Chunk = Bytes> + Default + 'static,
        ResB: BodyStream<Chunk = Bytes> + 'static,
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

    impl<S, Req, ResB> Service<Req> for EraserService<EraseResBody, S>
    where
        S: Service<Req, Response = WebResponse<ResB>>,
        ResB: BodyStream<Chunk = Bytes> + 'static,
    {
        type Response = WebResponse;
        type Error = S::Error;

        #[inline]
        async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
            let res = self.service.call(req).await?;
            Ok(res.map(ResponseBody::box_stream))
        }
    }

    impl<'r, C, B, S> Service<WebContext<'r, C, B>> for EraserService<EraseErr, S>
    where
        S: Service<WebContext<'r, C, B>>,
        S::Error: Into<Error>,
    {
        type Response = S::Response;
        type Error = Error;

        #[inline]
        async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
            self.service.call(ctx).await.map_err(Into::into)
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
}

#[cfg(test)]
mod test {
    use xitca_http::body::Once;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        App, WebContext,
        bytes::Bytes,
        error::Error,
        handler::handler_service,
        http::{Request, StatusCode, WebResponse},
        middleware::Group,
        service::ServiceExt,
    };

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
            // erase the body type to make it WebResponse type again.
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

    #[test]
    fn erase_error() {
        async fn middleware_fn<S, C, B, Err>(s: &S, ctx: WebContext<'_, C, B>) -> Result<WebResponse, StatusCode>
        where
            S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Err>,
        {
            s.call(ctx).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }

        async fn middleware_fn2<S, C, B>(s: &S, ctx: WebContext<'_, C, B>) -> Result<WebResponse, Error>
        where
            S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Error>,
        {
            s.call(ctx).await
        }

        let _ = App::new()
            // map WebResponse to WebResponse<Once<Bytes>> type.
            .at("/", handler_service(handler).enclosed(TypeEraser::error()))
            .enclosed(
                Group::new()
                    .enclosed_fn(middleware_fn)
                    .enclosed(TypeEraser::error())
                    .enclosed_fn(middleware_fn2),
            )
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();
    }
}
