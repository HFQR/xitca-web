//! limitation middleware.

use std::{
    cell::RefCell,
    pin::Pin,
    task::{Context, Poll, ready},
};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use xitca_http::Request;

use crate::{
    body::BodyStream,
    context::WebContext,
    error::{BodyError, BodyOverFlow},
    service::{Service, ready::ReadyService},
};

/// General purposed limitation middleware. Limiting request/response body size etc.
///
/// # Type mutation
/// [`Limit`] would mutate request body type from `B` to [`Limit<B>`]. Service enclosed by it must be
/// able to handle it's mutation or utilize [`TypeEraser`] to erase the mutation.
/// For more explanation please reference [`type mutation`](crate::middleware#type-mutation).
///
/// [`TypeEraser`]: crate::middleware::eraser::TypeEraser
#[derive(Copy, Clone)]
pub struct Limit {
    request_body_size: usize,
}

impl Default for Limit {
    fn default() -> Self {
        Self::new()
    }
}

impl Limit {
    pub const fn new() -> Self {
        Self {
            request_body_size: usize::MAX,
        }
    }

    /// Set max size in byte unit the request body can be.
    pub fn set_request_body_max_size(mut self, size: usize) -> Self {
        self.request_body_size = size;
        self
    }
}

impl<S, E> Service<Result<S, E>> for Limit {
    type Response = LimitService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| LimitService { service, limit: *self })
    }
}

pub struct LimitService<S> {
    service: S,
    limit: Limit,
}

impl<'r, S, C, B, Res, Err> Service<WebContext<'r, C, B>> for LimitService<S>
where
    B: BodyStream + Default,
    S: for<'r2> Service<WebContext<'r2, C, LimitBody<B>>, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = Err;

    async fn call(&self, mut ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let (parts, ext) = ctx.take_request().into_parts();
        let state = ctx.ctx;
        let (ext, body) = ext.replace_body(());
        let mut body = RefCell::new(LimitBody::new(body, self.limit.request_body_size));
        let mut req = Request::from_parts(parts, ext);

        self.service
            .call(WebContext::new(&mut req, &mut body, state))
            .await
            .inspect_err(|_| {
                let body = body.into_inner().into_inner();
                *ctx.body_borrow_mut() = body;
            })
    }
}

impl<S> ReadyService for LimitService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.service.ready().await
    }
}

pin_project! {
    pub struct LimitBody<B> {
        limit: usize,
        record: usize,
        #[pin]
        body: B
    }
}

impl<B: Default> Default for LimitBody<B> {
    fn default() -> Self {
        Self {
            limit: 0,
            record: 0,
            body: B::default(),
        }
    }
}

impl<B> LimitBody<B> {
    const fn new(body: B, limit: usize) -> Self {
        Self { limit, record: 0, body }
    }

    fn into_inner(self) -> B {
        self.body
    }
}

impl<B> Stream for LimitBody<B>
where
    B: BodyStream,
{
    type Item = Result<B::Chunk, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        if *this.record >= *this.limit {
            // search error module for downcast_ref::<BodyOverFlow>() before considering change the
            // error type.
            return Poll::Ready(Some(Err(BodyError::from(BodyOverFlow { limit: *this.limit }))));
        }

        match ready!(this.body.poll_next(cx)) {
            Some(res) => {
                let chunk = res.map_err(Into::into)?;
                *this.record += chunk.as_ref().len();
                // TODO: for now there is no way to split a chunk if it goes beyond body limit.
                Poll::Ready(Some(Ok(chunk)))
            }
            None => Poll::Ready(None),
        }
    }
}

#[cfg(test)]
mod test {
    use core::{future::poll_fn, pin::pin};

    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        App,
        body::BoxBody,
        bytes::Bytes,
        handler::{body::Body, handler_service},
        http::{StatusCode, WebRequest},
        test::collect_body,
    };

    use super::*;

    const CHUNK: &[u8] = b"hello,world!";

    async fn handler<B: BodyStream>(Body(body): Body<B>) -> String {
        let mut body = pin!(body);

        let chunk = poll_fn(|cx| body.as_mut().poll_next(cx)).await.unwrap().ok().unwrap();

        let err = poll_fn(|cx| body.as_mut().poll_next(cx)).await.unwrap().err().unwrap();
        let err = crate::error::Error::from(err.into());
        assert_eq!(
            err.to_string(),
            format!("body size reached limit: {} bytes", CHUNK.len())
        );

        let mut ctx = WebContext::new_test(());
        let res = err.call(ctx.as_web_ctx()).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        std::str::from_utf8(chunk.as_ref()).unwrap().to_string()
    }

    #[test]
    fn request_body_over_limit() {
        use futures_util::stream::{self, StreamExt};

        let item = || async { Ok::<_, BodyError>(Bytes::from_static(CHUNK)) };

        let body = stream::once(item()).chain(stream::once(item()));
        let req = WebRequest::default().map(|ext| ext.map_body(|_: ()| BoxBody::new(body).into()));

        let body = App::new()
            .at("/", handler_service(handler))
            .enclosed(Limit::new().set_request_body_max_size(CHUNK.len()))
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .ok()
            .unwrap()
            .into_body();

        let body = collect_body(body).now_or_panic().unwrap();

        assert_eq!(body, CHUNK);
    }
}
