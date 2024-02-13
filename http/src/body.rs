//! HTTP body types.
//!
//! body types are generic over [Stream] trait and mutation of body type must also implement said
//! trait for being accepted as body type that xitca-http know of.
//!
//! When implementing customized body type please reference [none_body_hint] and [exact_body_hint]
//! for contract of inferring body size with [Stream::size_hint] trait method.

use core::{
    convert::Infallible,
    marker::PhantomData,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use std::{borrow::Cow, error};

use futures_core::stream::{LocalBoxStream, Stream};
use pin_project_lite::pin_project;

use super::{
    bytes::{Buf, Bytes, BytesMut},
    error::BodyError,
};

// this is a crate level hack to hint for none body type.
// A body type with this size hint means the body MUST not be polled/collected by anyone.
pub const fn none_body_hint() -> (usize, Option<usize>) {
    NONE_BODY_HINT
}

pub const NONE_BODY_HINT: (usize, Option<usize>) = (usize::MAX, Some(0));

// this is a crate level hack to hint for exact body type.
// A body type with this size hint means the body MUST be polled/collected for exact length of usize.
pub const fn exact_body_hint(size: usize) -> (usize, Option<usize>) {
    (size, Some(size))
}

/// A unified request body type for different http protocols.
/// This enables one service type to handle multiple http protocols.
#[derive(Default)]
pub enum RequestBody {
    #[cfg(feature = "http1")]
    H1(super::h1::RequestBody),
    #[cfg(feature = "http2")]
    H2(super::h2::RequestBody),
    #[cfg(feature = "http3")]
    H3(super::h3::RequestBody),
    Unknown(BoxBody),
    #[default]
    None,
}

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            #[cfg(feature = "http1")]
            Self::H1(body) => Pin::new(body).poll_next(cx).map_err(Into::into),
            #[cfg(feature = "http2")]
            Self::H2(body) => Pin::new(body).poll_next(cx),
            #[cfg(feature = "http3")]
            Self::H3(body) => Pin::new(body).poll_next(cx),
            Self::Unknown(body) => Pin::new(body).poll_next(cx),
            Self::None => Poll::Ready(None),
        }
    }
}

impl<B> From<NoneBody<B>> for RequestBody {
    fn from(_: NoneBody<B>) -> Self {
        Self::None
    }
}

impl From<Bytes> for RequestBody {
    fn from(bytes: Bytes) -> Self {
        Self::from(Once::new(bytes))
    }
}

impl From<Once<Bytes>> for RequestBody {
    fn from(once: Once<Bytes>) -> Self {
        Self::from(BoxBody::new(once))
    }
}

impl From<BoxBody> for RequestBody {
    fn from(body: BoxBody) -> Self {
        Self::Unknown(body)
    }
}

macro_rules! req_bytes_impl {
    ($ty: ty) => {
        impl From<$ty> for RequestBody {
            fn from(item: $ty) -> Self {
                Self::from(Bytes::from(item))
            }
        }
    };
}

req_bytes_impl!(&'static [u8]);
req_bytes_impl!(Box<[u8]>);
req_bytes_impl!(Vec<u8>);
req_bytes_impl!(String);

/// None body type.
/// B type is used to infer other types of body's output type used together with NoneBody.
pub struct NoneBody<B>(PhantomData<fn(B)>);

impl<B> Default for NoneBody<B> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<B> Stream for NoneBody<B> {
    type Item = Result<B, Infallible>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unreachable!("NoneBody must not be polled. See NoneBody for detail")
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        none_body_hint()
    }
}

/// Full body type that can only be polled once with [Stream::poll_next].
#[derive(Default)]
pub struct Once<B>(Option<B>);

impl<B> Once<B>
where
    B: Buf + Unpin,
{
    #[inline]
    pub const fn new(body: B) -> Self {
        Self(Some(body))
    }
}

impl<B> From<B> for Once<B>
where
    B: Buf + Unpin,
{
    fn from(b: B) -> Self {
        Self::new(b)
    }
}

impl<B> Stream for Once<B>
where
    B: Buf + Unpin,
{
    type Item = Result<B, Infallible>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(mem::replace(self.get_mut(), Self(None)).0.map(Ok))
    }

    // use the length of buffer as both lower bound and upperbound.
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.0 {
            Some(ref b) => exact_body_hint(b.remaining()),
            None => unreachable!("Once must check size_hint before it got polled"),
        }
    }
}

pin_project! {
    #[project = EitherProj]
    pub enum Either<L, R> {
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
        Self::L { inner }
    }

    #[inline]
    pub const fn right(inner: R) -> Self {
        Self::R { inner }
    }
}

impl<L, R, T, E, E2> Stream for Either<L, R>
where
    L: Stream<Item = Result<T, E>>,
    R: Stream<Item = Result<T, E2>>,
    E2: From<E>,
{
    type Item = Result<T, E2>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.project() {
            EitherProj::L { inner } => inner.poll_next(cx).map(|res| res.map(|res| res.map_err(Into::into))),
            EitherProj::R { inner } => inner.poll_next(cx),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match *self {
            Self::L { ref inner } => inner.size_hint(),
            Self::R { ref inner } => inner.size_hint(),
        }
    }
}

/// type erased stream body.
pub struct BoxBody(LocalBoxStream<'static, Result<Bytes, BodyError>>);

impl Default for BoxBody {
    fn default() -> Self {
        Self::new(NoneBody::default())
    }
}

impl BoxBody {
    #[inline]
    pub fn new<B, E>(body: B) -> Self
    where
        B: Stream<Item = Result<Bytes, E>> + 'static,
        E: Into<BodyError>,
    {
        Self(Box::pin(BoxStreamMapErr { body }))
    }
}

impl Stream for BoxBody {
    type Item = Result<Bytes, BodyError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().0.as_mut().poll_next(cx)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

pin_project! {
    struct BoxStreamMapErr<B> {
        #[pin]
        body: B
    }
}

impl<B, T, E> Stream for BoxStreamMapErr<B>
where
    B: Stream<Item = Result<T, E>>,
    E: Into<BodyError>,
{
    type Item = Result<T, BodyError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().body.poll_next(cx).map_err(Into::into)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.body.size_hint()
    }
}

pin_project! {
    /// A unified response body type.
    /// Generic type is for custom pinned response body(type implement [Stream](futures_core::Stream)).
    pub struct ResponseBody<B = BoxBody> {
        #[pin]
        inner: ResponseBodyInner<B>
    }
}

pin_project! {
    #[project = ResponseBodyProj]
    #[project_replace = ResponseBodyProjReplace]
    enum ResponseBodyInner<B> {
        None,
        Bytes {
            bytes: Bytes,
        },
        Stream {
            #[pin]
            stream: B,
        },
    }
}

impl<B> Default for ResponseBody<B> {
    fn default() -> Self {
        Self::none()
    }
}

impl ResponseBody {
    /// Construct a new Stream variant of ResponseBody with default type as [BoxBody]
    #[inline]
    pub fn box_stream<B, E>(stream: B) -> Self
    where
        B: Stream<Item = Result<Bytes, E>> + 'static,
        E: Into<BodyError>,
    {
        Self::stream(BoxBody::new(stream))
    }
}

impl<B> ResponseBody<B> {
    /// indicate no body is attached to response.
    /// `content-length` and `transfer-encoding` headers would not be added to
    /// response when [BodySize] is used for inferring response body type.
    #[inline]
    pub const fn none() -> Self {
        Self {
            inner: ResponseBodyInner::None,
        }
    }

    /// indicate empty body is attached to response.
    /// `content-length: 0` header would be added to response when [BodySize] is
    /// used for inferring response body type.
    #[inline]
    pub const fn empty() -> Self {
        Self {
            inner: ResponseBodyInner::Bytes { bytes: Bytes::new() },
        }
    }

    /// Construct a new Stream variant of ResponseBody
    #[inline]
    pub fn stream(stream: B) -> Self {
        Self {
            inner: ResponseBodyInner::Stream { stream },
        }
    }

    /// Construct a new Bytes variant of ResponseBody
    #[inline]
    pub fn bytes<B2>(bytes: B2) -> Self
    where
        Bytes: From<B2>,
    {
        Self {
            inner: ResponseBodyInner::Bytes {
                bytes: Bytes::from(bytes),
            },
        }
    }

    /// erase generic body type by boxing the variant.
    #[inline]
    pub fn into_boxed<E>(self) -> ResponseBody
    where
        B: Stream<Item = Result<Bytes, E>> + 'static,
        E: error::Error + Send + Sync + 'static,
    {
        match self.inner {
            ResponseBodyInner::None => ResponseBody::none(),
            ResponseBodyInner::Bytes { bytes } => ResponseBody::bytes(bytes),
            ResponseBodyInner::Stream { stream } => ResponseBody::box_stream(stream),
        }
    }
}

impl<B, E> Stream for ResponseBody<B>
where
    B: Stream<Item = Result<Bytes, E>>,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut inner = self.project().inner;
        match inner.as_mut().project() {
            ResponseBodyProj::None => Poll::Ready(None),
            ResponseBodyProj::Bytes { .. } => match inner.project_replace(ResponseBodyInner::None) {
                ResponseBodyProjReplace::Bytes { bytes } => Poll::Ready(Some(Ok(bytes))),
                _ => unreachable!(),
            },
            ResponseBodyProj::Stream { stream } => stream.poll_next(cx),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.inner {
            ResponseBodyInner::None => none_body_hint(),
            ResponseBodyInner::Bytes { ref bytes } => exact_body_hint(bytes.len()),
            ResponseBodyInner::Stream { ref stream } => stream.size_hint(),
        }
    }
}

impl<B> From<NoneBody<B>> for ResponseBody {
    fn from(_: NoneBody<B>) -> Self {
        ResponseBody::none()
    }
}

impl<B> From<Once<B>> for ResponseBody
where
    B: Into<Bytes>,
{
    fn from(once: Once<B>) -> Self {
        ResponseBody::bytes(once.0.map(Into::into).unwrap_or_default())
    }
}

impl From<BoxBody> for ResponseBody {
    fn from(stream: BoxBody) -> Self {
        Self::stream(stream)
    }
}

macro_rules! res_bytes_impl {
    ($ty: ty) => {
        impl<B> From<$ty> for ResponseBody<B> {
            fn from(item: $ty) -> Self {
                Self::bytes(item)
            }
        }
    };
}

res_bytes_impl!(Bytes);
res_bytes_impl!(BytesMut);
res_bytes_impl!(&'static [u8]);
res_bytes_impl!(&'static str);
res_bytes_impl!(Box<[u8]>);
res_bytes_impl!(Vec<u8>);
res_bytes_impl!(String);

impl<B> From<Box<str>> for ResponseBody<B> {
    fn from(str: Box<str>) -> Self {
        Self::from(Box::<[u8]>::from(str))
    }
}

impl<B> From<Cow<'static, str>> for ResponseBody<B> {
    fn from(str: Cow<'static, str>) -> Self {
        match str {
            Cow::Owned(str) => Self::from(str),
            Cow::Borrowed(str) => Self::from(str),
        }
    }
}

/// Body size hint.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BodySize {
    /// Absence of body can be assumed from method or status code.
    ///
    /// Will skip writing Content-Length header.
    None,
    /// Known size body.
    ///
    /// Will write `Content-Length: N` header.
    Sized(usize),
    /// Unknown size body.
    ///
    /// Will not write Content-Length header. Can be used with chunked Transfer-Encoding.
    Stream,
}

impl BodySize {
    #[inline]
    pub fn from_stream<S>(stream: &S) -> Self
    where
        S: Stream,
    {
        match stream.size_hint() {
            NONE_BODY_HINT => Self::None,
            (_, Some(size)) => Self::Sized(size),
            (_, None) => Self::Stream,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn stream_body_size_hint() {
        let body = BoxBody::new(Once::new(Bytes::new()));
        assert_eq!(BodySize::from_stream(&body), BodySize::Sized(0));

        let body = BoxBody::new(NoneBody::<Bytes>::default());
        assert_eq!(BodySize::from_stream(&body), BodySize::None);
    }
}
