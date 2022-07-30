use std::{
    convert::Infallible,
    marker::PhantomData,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

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

const NONE_BODY_HINT: (usize, Option<usize>) = (usize::MAX, Some(0));

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
    #[default]
    None,
}

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            #[cfg(feature = "http1")]
            Self::H1(body) => Pin::new(body).poll_next(_cx),
            #[cfg(feature = "http2")]
            Self::H2(body) => Pin::new(body).poll_next(_cx),
            #[cfg(feature = "http3")]
            Self::H3(body) => Pin::new(body).poll_next(_cx),
            Self::None => Poll::Ready(None),
        }
    }
}

/// None body type.
/// B type is used to infer other types of body's output type used together with Empty.
pub struct NoneBody<B>(PhantomData<B>);

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

pub type StreamBody = LocalBoxStream<'static, Result<Bytes, BodyError>>;

pin_project! {
    /// A unified response body type.
    /// Generic type is for custom pinned response body(type implement [Stream](futures_core::Stream)).
    #[project = ResponseBodyProj]
    #[project_replace = ResponseBodyProjReplace]
    pub enum ResponseBody<B = StreamBody> {
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

impl<B, E> ResponseBody<B>
where
    B: Stream<Item = Result<Bytes, E>>,
{
    pub fn is_eof(&self) -> bool {
        match *self {
            Self::None => true,
            Self::Bytes { ref bytes, .. } => bytes.is_empty(),
            Self::Stream { .. } => false,
        }
    }

    /// Construct a new Stream variant of ResponseBody
    #[inline]
    pub fn stream(stream: B) -> Self {
        Self::Stream { stream }
    }

    /// Construct a new Bytes variant of ResponseBody
    #[inline]
    pub fn bytes(bytes: Bytes) -> Self {
        Self::Bytes { bytes }
    }

    /// Drop [ResponseBody::Stream] variant to cast it to given generic B1 stream type.
    ///
    /// # Note:
    /// Response's HeaderMap may need according change when Stream variant is droped.
    pub fn drop_stream_cast<B1>(self) -> ResponseBody<B1> {
        match self {
            Self::None | Self::Stream { .. } => ResponseBody::None,
            Self::Bytes { bytes } => ResponseBody::Bytes { bytes },
        }
    }
}

impl<B, E> Stream for ResponseBody<B>
where
    B: Stream<Item = Result<Bytes, E>>,
{
    type Item = Result<Bytes, E>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.as_mut().project() {
            ResponseBodyProj::None => Poll::Ready(None),
            ResponseBodyProj::Bytes { .. } => match self.project_replace(ResponseBody::None) {
                ResponseBodyProjReplace::Bytes { bytes } => Poll::Ready(Some(Ok(bytes))),
                _ => unreachable!(),
            },
            ResponseBodyProj::Stream { stream } => stream.poll_next(cx),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::None => none_body_hint(),
            Self::Bytes { ref bytes } => exact_body_hint(bytes.len()),
            Self::Stream { ref stream } => stream.size_hint(),
        }
    }
}

impl From<StreamBody> for ResponseBody {
    fn from(stream: StreamBody) -> Self {
        Self::Stream { stream }
    }
}

impl<B> From<Bytes> for ResponseBody<B> {
    fn from(bytes: Bytes) -> Self {
        Self::Bytes { bytes }
    }
}

impl<B> From<BytesMut> for ResponseBody<B> {
    fn from(bytes: BytesMut) -> Self {
        Self::Bytes { bytes: bytes.freeze() }
    }
}

macro_rules! from_impl {
    ($ty: ty) => {
        impl<B> From<$ty> for ResponseBody<B> {
            fn from(item: $ty) -> Self {
                Self::Bytes {
                    bytes: Bytes::from(item),
                }
            }
        }
    };
}

from_impl!(&'static [u8]);
from_impl!(Vec<u8>);
from_impl!(String);

impl<B> From<&'_ str> for ResponseBody<B> {
    fn from(str: &str) -> Self {
        Self::Bytes {
            bytes: Bytes::copy_from_slice(str.as_bytes()),
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
