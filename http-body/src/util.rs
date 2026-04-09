use core::{
    convert::Infallible,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Buf;
use futures_core::stream::Stream;
use http::header::HeaderMap;
use pin_project_lite::pin_project;

use super::{body::Body, frame::Frame, size_hint::SizeHint};

pin_project! {
    /// A body that consists of a single chunk.
    #[derive(Clone, Copy, Debug)]
    pub struct Full<D> {
        data: Option<D>,
    }
}

impl<D> Full<D>
where
    D: Buf,
{
    /// Create a new `Full`.
    #[inline]
    pub const fn new(data: D) -> Self {
        Self { data: Some(data) }
    }
}

impl<D> Body for Full<D>
where
    D: Buf,
{
    type Data = D;
    type Error = Infallible;

    #[inline]
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<crate::Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.data.take().map(|d| Ok(Frame::Data(d))))
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.data.is_none()
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        match self.data {
            Some(ref data) => SizeHint::exact(data.remaining()),
            None => SizeHint::None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Empty<D>(PhantomData<fn(D)>);

impl<D> Empty<D> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<D> Body for Empty<D> {
    type Data = D;
    type Error = Infallible;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(None)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        true
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        SizeHint::None
    }
}

pin_project! {
    pub struct Either<L, R> {
        #[pin]
        inner: EitherInner<L, R>
    }
}

pin_project! {
    #[project = EitherProj]
    enum EitherInner<L, R> {
        L {
            #[pin]
            inner: L
        },
        R {
            #[pin]
            inner: R
        }
    }
}

impl<L, R> Either<L, R> {
    #[inline]
    pub const fn left(inner: L) -> Self {
        Self {
            inner: EitherInner::L { inner },
        }
    }

    #[inline]
    pub const fn right(inner: R) -> Self {
        Self {
            inner: EitherInner::R { inner },
        }
    }

    #[inline]
    pub fn into_left(self) -> Result<L, Self> {
        match self.inner {
            EitherInner::L { inner } => Ok(inner),
            inner => Err(Self { inner }),
        }
    }

    #[inline]
    pub fn into_right(self) -> Result<R, Self> {
        match self.inner {
            EitherInner::R { inner } => Ok(inner),
            inner => Err(Self { inner }),
        }
    }
}

impl<L> Either<L, L> {
    #[inline]
    pub fn into_inner(self) -> L {
        match self.inner {
            EitherInner::L { inner } => inner,
            EitherInner::R { inner } => inner,
        }
    }
}

impl<L, R> Body for Either<L, R>
where
    L: Body,
    R: Body<Data = L::Data>,
    R::Error: From<L::Error>,
{
    type Data = L::Data;
    type Error = R::Error;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.project().inner.project() {
            EitherProj::L { inner } => inner.poll_frame(cx).map(|res| res.map(|res| res.map_err(Into::into))),
            EitherProj::R { inner } => inner.poll_frame(cx),
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        match self.inner {
            EitherInner::L { ref inner } => inner.is_end_stream(),
            EitherInner::R { ref inner } => inner.is_end_stream(),
        }
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        match self.inner {
            EitherInner::L { ref inner } => inner.size_hint(),
            EitherInner::R { ref inner } => inner.size_hint(),
        }
    }
}

pin_project! {
    /// Bidirectional adapter between [`Stream`] and [`Body`] at the [`Frame`] level:
    /// - When the inner value implements [`Body`], this type implements [`Stream`] yielding
    ///   `Result<Frame<Body::Data>, Body::Error>`.
    /// - When the inner value implements [`Stream`], this type implements [`Body`] forwarding
    ///   each `Result<Frame<D>, E>` frame as-is.
    #[derive(Debug)]
    pub struct StreamBody<T> {
        #[pin]
        pub(crate) value: T
    }
}

impl<T> StreamBody<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<S, D, E> Body for StreamBody<S>
where
    S: Stream<Item = Result<Frame<D>, E>>,
{
    type Data = D;
    type Error = E;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Stream::poll_next(self.project().value, cx)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        false
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        match Stream::size_hint(&self.value) {
            (low, Some(up)) if low == up => SizeHint::exact(low),
            SizeHint::NO_BODY_HINT => SizeHint::None,
            _ => SizeHint::Unknown,
        }
    }
}

pin_project! {
    /// Bidirectional adapter between [`Stream`] and [`Body`]:
    /// - When the inner value implements [`Body`], this type implements [`Stream`] yielding
    ///   `Result<Body::Data, Body::Error>`. Trailer frames are ignored and treated as end of stream.
    /// - When the inner value implements [`Stream`], this type implements [`Body`] wrapping each
    ///   streamed item into [`Frame::Data`].
    #[derive(Debug)]
    pub struct StreamDataBody<T> {
        #[pin]
        pub(crate) value: T
    }
}

impl<T> StreamDataBody<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<S, T, E> Body for StreamDataBody<S>
where
    S: Stream<Item = Result<T, E>>,
{
    type Data = T;
    type Error = E;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Stream::poll_next(self.project().value, cx).map_ok(Frame::Data)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        false
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        SizeHint::from_stream_size_hint(self.value.size_hint())
    }
}

pin_project! {
    pub struct Data<B> {
        #[pin]
        body: B
    }
}

impl<B> Data<B>
where
    B: Body,
{
    pub fn new(body: B) -> Self {
        Self { body }
    }
}

impl<B> Body for Data<B>
where
    B: Body,
{
    type Data = B::Data;
    type Error = B::Error;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.project().body.poll_frame(cx) {
            Poll::Ready(Some(Ok(Frame::Trailers(_)))) => Poll::Ready(None),
            res => res,
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

pub struct Trailers<D> {
    trailers: Option<HeaderMap>,
    _data: PhantomData<fn(D)>,
}

impl<D> Trailers<D> {
    pub fn new(trailers: HeaderMap) -> Self {
        Self {
            trailers: Some(trailers),
            _data: PhantomData,
        }
    }
}

impl<D> Body for Trailers<D> {
    type Data = D;
    type Error = Infallible;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(
            self.get_mut()
                .trailers
                .take()
                .map(|trailers| Ok(Frame::Trailers(trailers))),
        )
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.trailers.is_none()
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        SizeHint::None
    }
}
