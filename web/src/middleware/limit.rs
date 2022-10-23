use std::{
    cell::RefCell,
    convert::Infallible,
    error, fmt,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;

use crate::{
    dev::service::{pipeline::PipelineE, ready::ReadyService, Service},
    handler::Responder,
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, status::StatusCode},
    request::WebRequest,
    response::WebResponse,
    stream::WebStream,
};

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
    pub fn new() -> Self {
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

impl<S> Service<S> for Limit {
    type Response = LimitService<S>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f;

    fn call<'s>(&'s self, service: S) -> Self::Future<'s>
    where
        S: 's,
    {
        async { Ok(LimitService { service, limit: *self }) }
    }
}

pub struct LimitService<S> {
    service: S,
    limit: Limit,
}

pub type LimitServiceError<E> = PipelineE<LimitError, E>;

impl<'r, S, C, B, Res, Err> Service<WebRequest<'r, C, B>> for LimitService<S>
where
    C: 'r,
    B: WebStream + Default + 'r,
    S: for<'rs> Service<WebRequest<'rs, C, LimitBody<B>>, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = LimitServiceError<Err>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, 'r: 'f;

    fn call<'s>(&'s self, mut req: WebRequest<'r, C, B>) -> Self::Future<'s>
    where
        'r: 's,
    {
        async move {
            let (mut http_req, body) = req.take_request().replace_body(());
            let mut body = RefCell::new(LimitBody::new(body, self.limit.request_body_size));

            let req = WebRequest::new(&mut http_req, &mut body, req.ctx);

            self.service.call(req).await.map_err(LimitServiceError::Second)
        }
    }
}

impl<S> ReadyService for LimitService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
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
    fn new(body: B, limit: usize) -> Self {
        Self { limit, record: 0, body }
    }
}

impl<B> Stream for LimitBody<B>
where
    B: WebStream,
{
    type Item = Result<B::Chunk, LimitBodyError<B::Error>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        if *this.record >= *this.limit {
            return Poll::Ready(Some(Err(LimitBodyError::First(LimitError::BodyOverSize(*this.limit)))));
        }

        match ready!(this.body.poll_next(cx)) {
            Some(res) => {
                let chunk = res.map_err(LimitBodyError::Second)?;
                *this.record += chunk.as_ref().len();
                // TODO: for now there is no way to split a chunk if it goes beyond body limit.
                Poll::Ready(Some(Ok(chunk)))
            }
            None => Poll::Ready(None),
        }
    }
}

pub type LimitBodyError<E> = PipelineE<LimitError, E>;

#[derive(Debug)]
pub enum LimitError {
    BodyOverSize(usize),
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::BodyOverSize(size) => write!(f, "Body size reached limit: {} bytes.", size),
        }
    }
}

impl error::Error for LimitError {}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for LimitError {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
        let mut res = req.into_response(format!("{self}"));
        res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
        *res.status_mut() = StatusCode::BAD_REQUEST;
        async { res }
    }
}

#[cfg(test)]
mod test {
    use std::future::poll_fn;

    use xitca_http::Request;
    use xitca_unsafe_collection::{futures::NowOrPanic, pin};

    use crate::{
        dev::bytes::Bytes,
        error::BodyError,
        handler::{body::Body, handler_service},
        response::StreamBody,
        test::collect_body,
        App,
    };

    use super::*;

    async fn handler<B: WebStream>(Body(body): Body<B>) -> String {
        pin!(body);

        let chunk = poll_fn(|cx| body.as_mut().poll_next(cx)).await.unwrap().unwrap();

        assert!(poll_fn(|cx| body.as_mut().poll_next(cx)).await.unwrap().is_err());

        std::str::from_utf8(chunk.as_ref()).unwrap().to_string()
    }

    #[test]
    fn request_body_over_limit() {
        use futures_util::stream::{self, StreamExt};

        let chunk = b"hello,world!";

        let item = || async { Ok::<_, BodyError>(Bytes::from_static(chunk)) };

        let body = stream::once(item()).chain(stream::once(item()));

        let req = Request::new(StreamBody::new(body));

        let body = App::new()
            .at("/", handler_service(handler))
            .enclosed(Limit::new().set_request_body_max_size(chunk.len()))
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

        assert_eq!(body, chunk);
    }
}
