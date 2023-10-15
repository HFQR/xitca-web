use std::{
    cell::RefCell,
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::Stream;
use http_body::{Body, SizeHint};
use pin_project_lite::pin_project;
use xitca_http::{
    body::{none_body_hint, BodySize},
    util::service::router::PathGen,
};
use xitca_unsafe_collection::fake_send_sync::{FakeSend, FakeSync};

use crate::{
    dev::{
        bytes::Buf,
        service::{ready::ReadyService, Service},
    },
    http::{header::HeaderMap, Request, RequestExt, Response},
    request::WebRequest,
    response::WebResponse,
};

pub struct TowerHttpCompat<S> {
    service: S,
}

impl<S> TowerHttpCompat<S> {
    pub fn new(service: S) -> Self
    where
        S: Clone,
    {
        Self { service }
    }
}

impl<S> Service for TowerHttpCompat<S>
where
    S: Clone,
{
    type Response = TowerCompatService<S>;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        let service = self.service.clone();

        Ok(TowerCompatService {
            service: RefCell::new(service),
        })
    }
}

impl<S> PathGen for TowerHttpCompat<S> {}

pub struct TowerCompatService<S> {
    service: RefCell<S>,
}

impl<S> TowerCompatService<S> {
    pub fn new(service: S) -> Self {
        Self {
            service: RefCell::new(service),
        }
    }
}

impl<'r, C, ReqB, S, ResB> Service<WebRequest<'r, C, ReqB>> for TowerCompatService<S>
where
    S: tower_service::Service<Request<CompatBody<FakeSend<RequestExt<ReqB>>>>, Response = Response<ResB>>,
    ResB: Body,
    C: Clone + 'static,
    ReqB: Default,
{
    type Response = WebResponse<CompatBody<ResB>>;
    type Error = S::Error;

    async fn call(&self, mut req: WebRequest<'r, C, ReqB>) -> Result<Self::Response, Self::Error> {
        let ctx = req.state().clone();
        let (mut parts, ext) = req.take_request().into_parts();
        parts.extensions.insert(FakeSync::new(FakeSend::new(ctx)));
        let req = Request::from_parts(parts, CompatBody::new(FakeSend::new(ext)));
        let fut = tower_service::Service::call(&mut *self.service.borrow_mut(), req);
        fut.await.map(|res| res.map(CompatBody::new))
    }
}

impl<S> ReadyService for TowerCompatService<S> {
    type Ready = ();

    #[inline]
    async fn ready(&self) -> Self::Ready {}
}

pin_project! {
    pub struct CompatBody<B> {
        #[pin]
        body: B
    }
}

impl<B> CompatBody<B> {
    pub fn new(body: B) -> Self {
        Self { body }
    }

    pub fn into_inner(self) -> B {
        self.body
    }
}

impl<B, T, E> Body for CompatBody<B>
where
    B: Stream<Item = Result<T, E>>,
    T: Buf,
{
    type Data = T;
    type Error = E;

    #[inline]
    fn poll_data(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        self.project().body.poll_next(cx)
    }

    #[inline]
    fn poll_trailers(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<Option<HeaderMap>, Self::Error>> {
        Poll::Ready(Ok(None))
    }

    fn size_hint(&self) -> SizeHint {
        let mut hint = SizeHint::new();
        match BodySize::from_stream(&self.body) {
            BodySize::None => {
                let (low, upper) = none_body_hint();
                hint.set_lower(low as u64);
                hint.set_upper(upper.unwrap() as u64);
            }
            BodySize::Sized(size) => hint.set_exact(size as u64),
            BodySize::Stream => {}
        }

        hint
    }
}

impl<B> Stream for CompatBody<B>
where
    B: Body,
{
    type Item = Result<B::Data, B::Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().body.poll_data(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let hint = self.body.size_hint();
        (hint.lower() as usize, hint.upper().map(|num| num as usize))
    }
}

#[cfg(test)]
mod test {
    use xitca_http::body::{exact_body_hint, Once};

    use crate::dev::bytes::Bytes;

    use super::*;

    #[test]
    fn body_compat() {
        let buf = Bytes::from_static(b"996");
        let len = buf.len();
        let body = CompatBody::new(Once::new(buf));

        let size = Body::size_hint(&body);

        assert_eq!(
            (size.lower() as usize, size.upper().map(|num| num as usize)),
            exact_body_hint(len)
        );

        let body = CompatBody::new(body);

        let size = Stream::size_hint(&body);

        assert_eq!(size, exact_body_hint(len));
    }
}
