use core::{
    cell::RefCell,
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};

use std::borrow::Cow;

use pin_project_lite::pin_project;
use xitca_http::util::service::router::{PathGen, RouteGen, RouterMapErr};
use xitca_unsafe_collection::fake::{FakeSend, FakeSync};

use crate::{
    body::{Body, Frame, ResponseBody, SizeHint},
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
    ResB: http_body::Body,
    C: Clone + 'static,
    ReqB: Default,
{
    type Response = WebResponse<CompatBody<ResB>>;
    type Error = S::Error;

    async fn call(&self, mut ctx: WebContext<'r, C, ReqB>) -> Result<Self::Response, Self::Error> {
        let (parts, ext) = ctx.take_request().into_parts();
        let ctx = ctx.state().clone();
        let req = Request::from_parts(parts, CompatReqBody::new(ext, ctx));
        let fut = tower_service::Service::call(&mut *self.0.borrow_mut(), req);
        fut.await.map(|res| res.map(CompatBody::new))
    }
}

impl<S> ReadyService for TowerCompatService<S> {
    type Ready = ();

    #[inline]
    async fn ready(&self) -> Self::Ready {}
}

pub struct CompatReqBody<B, C> {
    body: FakeSend<CompatBody<B>>,
    ctx: FakeSend<FakeSync<C>>,
}

impl<B, C> CompatReqBody<B, C> {
    #[inline]
    pub fn new(body: B, ctx: C) -> Self {
        Self {
            body: FakeSend::new(CompatBody::new(body)),
            ctx: FakeSend::new(FakeSync::new(ctx)),
        }
    }

    /// destruct compat body into owned value of body and state context
    ///
    /// # Panics
    /// - When called from a thread not where B is originally constructed.
    #[inline]
    pub fn into_parts(self) -> (B, C) {
        (self.body.into_inner().into_inner(), self.ctx.into_inner().into_inner())
    }
}

impl<B, C> http_body::Body for CompatReqBody<B, C>
where
    B: Body + Unpin,
    C: Unpin,
    B::Data: Buf,
{
    type Data = B::Data;
    type Error = B::Error;

    #[inline]
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        http_body::Body::poll_frame(Pin::new(&mut *self.get_mut().body), cx)
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        http_body::Body::size_hint(&*self.body)
    }
}

pin_project! {
    #[derive(Default)]
    pub struct CompatBody<B> {
        #[pin]
        body: B
    }
}

impl<B> CompatBody<B> {
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
        impl<B> From<$ty> for CompatBody<B>
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

impl<B> http_body::Body for CompatBody<B>
where
    B: Body,
    B::Data: Buf,
{
    type Data = B::Data;
    type Error = B::Error;

    #[inline]
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Body::poll_frame(self.project().body, cx).map_ok(|frame| match frame {
            Frame::Data(data) => http_body::Frame::data(data),
            Frame::Trailers(trailers) => http_body::Frame::trailers(trailers),
        })
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        Body::is_end_stream(&self.body)
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        match Body::size_hint(&self.body) {
            SizeHint::Exact(size) => http_body::SizeHint::with_exact(size),
            _ => http_body::SizeHint::default(),
        }
    }
}

impl<B> Body for CompatBody<B>
where
    B: http_body::Body,
{
    type Data = B::Data;
    type Error = B::Error;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        http_body::Body::poll_frame(self.project().body, cx).map_ok(|frame| match frame.into_data() {
            Ok(data) => Frame::Data(data),
            Err(frame) => Frame::Trailers(frame.into_trailers().ok().unwrap()),
        })
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        http_body::Body::is_end_stream(&self.body)
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        match http_body::Body::size_hint(&self.body).exact() {
            Some(size) => SizeHint::Exact(size),
            None => SizeHint::Unknown,
        }
    }
}
