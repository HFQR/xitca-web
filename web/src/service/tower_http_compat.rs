use core::{
    cell::RefCell,
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll, ready},
};

use std::borrow::Cow;

use futures_core::stream::Stream;
use http_body::{Body, Frame, SizeHint};
use pin_project_lite::pin_project;
use xitca_http::{
    body::{BodySize, none_body_hint},
    util::service::router::{PathGen, RouteGen, RouterMapErr},
};
use xitca_unsafe_collection::fake::{FakeSend, FakeSync};

use crate::{
    body::ResponseBody,
    bytes::{Buf, Bytes, BytesMut},
    context::WebContext,
    http::{Request, RequestExt, Response, WebResponse},
    service::{Service, ready::ReadyService},
};

/// A middleware type that bridge `xitca-service` and `tower-service`.
/// Any `tower-http` type that impl [tower_service::Service] trait can be passed to it and used as xitca-web's service type.
pub struct TowerHttpCompat<S>(S);

impl<S> TowerHttpCompat<S> {
    pub const fn new(service: S) -> Self
    where
        S: Clone,
    {
        Self(service)
    }
}

impl<S> Service for TowerHttpCompat<S>
where
    S: Clone,
{
    type Response = TowerCompatService<S>;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        let service = self.0.clone();
        Ok(TowerCompatService(RefCell::new(service)))
    }
}

impl<S> PathGen for TowerHttpCompat<S> {}

impl<S> RouteGen for TowerHttpCompat<S> {
    type Route<R> = RouterMapErr<R>;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        RouterMapErr(route)
    }
}

pub struct TowerCompatService<S>(RefCell<S>);

impl<S> TowerCompatService<S> {
    pub const fn new(service: S) -> Self {
        Self(RefCell::new(service))
    }
}

impl<'r, C, ReqB, S, ResB> Service<WebContext<'r, C, ReqB>> for TowerCompatService<S>
where
    S: tower_service::Service<Request<CompatReqBody<RequestExt<ReqB>, C>>, Response = Response<ResB>>,
    ResB: Body,
    C: Clone + 'static,
    ReqB: Default,
{
    type Response = WebResponse<CompatResBody<ResB>>;
    type Error = S::Error;

    async fn call(&self, mut ctx: WebContext<'r, C, ReqB>) -> Result<Self::Response, Self::Error> {
        let (parts, ext) = ctx.take_request().into_parts();
        let ctx = ctx.state().clone();
        let req = Request::from_parts(parts, CompatReqBody::new(ext, ctx));
        let fut = tower_service::Service::call(&mut *self.0.borrow_mut(), req);
        fut.await.map(|res| res.map(CompatResBody::new))
    }
}

impl<S> ReadyService for TowerCompatService<S> {
    type Ready = ();

    #[inline]
    async fn ready(&self) -> Self::Ready {}
}

pub struct CompatReqBody<B, C> {
    body: FakeSend<B>,
    ctx: FakeSend<FakeSync<C>>,
}

impl<B, C> CompatReqBody<B, C> {
    #[inline]
    pub fn new(body: B, ctx: C) -> Self {
        Self {
            body: FakeSend::new(body),
            ctx: FakeSend::new(FakeSync::new(ctx)),
        }
    }

    /// destruct compat body into owned value of body and state context
    ///
    /// # Panics
    /// - When called from a thread not where B is originally constructed.
    #[inline]
    pub fn into_parts(self) -> (B, C) {
        (self.body.into_inner(), self.ctx.into_inner().into_inner())
    }
}

impl<B, C, T, E> Body for CompatReqBody<B, C>
where
    B: Stream<Item = Result<T, E>> + Unpin,
    C: Unpin,
    T: Buf,
{
    type Data = T;
    type Error = E;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut *self.get_mut().body).poll_next(cx).map_ok(Frame::data)
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        size_hint(BodySize::from_stream(&*self.body))
    }
}

pin_project! {
    #[derive(Default)]
    pub struct CompatResBody<B> {
        #[pin]
        body: B
    }
}

impl<B> CompatResBody<B> {
    pub const fn new(body: B) -> Self {
        Self { body }
    }

    pub fn into_inner(self) -> B {
        self.body
    }
}

// useful shortcuts conversion for B type.
macro_rules! impl_from {
    ($ty: ty) => {
        impl<B> From<$ty> for CompatResBody<B>
        where
            B: From<$ty>,
        {
            fn from(body: $ty) -> Self {
                Self::new(B::from(body))
            }
        }
    };
}

impl_from!(Bytes);
impl_from!(BytesMut);
impl_from!(&'static [u8]);
impl_from!(&'static str);
impl_from!(Box<[u8]>);
impl_from!(Vec<u8>);
impl_from!(String);
impl_from!(Box<str>);
impl_from!(Cow<'static, str>);
impl_from!(ResponseBody);

impl<B, T, E> Body for CompatResBody<B>
where
    B: Stream<Item = Result<T, E>>,
    T: Buf,
{
    type Data = T;
    type Error = E;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        self.project().body.poll_next(cx).map_ok(Frame::data)
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        size_hint(BodySize::from_stream(&self.body))
    }
}

impl<B> Stream for CompatResBody<B>
where
    B: Body,
{
    type Item = Result<B::Data, B::Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match ready!(self.project().body.poll_frame(cx)) {
            Some(res) => Poll::Ready(res.map(|frame| frame.into_data().ok()).transpose()),
            None => Poll::Ready(None),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let hint = self.body.size_hint();
        (hint.lower() as usize, hint.upper().map(|num| num as usize))
    }
}

fn size_hint(size: BodySize) -> SizeHint {
    let mut hint = SizeHint::new();
    match size {
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

#[cfg(test)]
mod test {
    use xitca_http::body::{Once, exact_body_hint};

    use super::*;

    #[test]
    fn body_compat() {
        let buf = Bytes::from_static(b"996");
        let len = buf.len();
        let body = CompatResBody::new(Once::new(buf));

        let size = Body::size_hint(&body);

        assert_eq!(
            (size.lower() as usize, size.upper().map(|num| num as usize)),
            exact_body_hint(len)
        );

        let body = CompatResBody::new(body);

        let size = Stream::size_hint(&body);

        assert_eq!(size, exact_body_hint(len));
    }
}
