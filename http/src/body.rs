//! HTTP body types.
//!
//! body types are generic over [Body] trait and mutation of body type must also implement said
//! trait for being accepted as body type that xitca-http know of.

pub use http_body_alt::{Body, BodyExt, Frame, SizeHint, util::*};

use core::{
    pin::Pin,
    task::{Context, Poll},
};

use std::{borrow::Cow, error};

use pin_project_lite::pin_project;

use super::{
    bytes::{Buf, Bytes, BytesMut},
    error::BodyError,
};

/// A unified request body type for different http protocols.
/// This enables one service type to handle multiple http protocols.
#[derive(Default)]
pub enum RequestBody {
    #[cfg(feature = "http2")]
    H2(super::h2::RequestBody),
    #[cfg(feature = "http3")]
    H3(super::h3::RequestBody),
    Boxed(BoxBody),
    #[default]
    None,
}

impl RequestBody {
    pub fn into_boxed(self) -> BoxBody {
        match self {
            #[cfg(feature = "http2")]
            Self::H2(body) => BoxBody::new(body),
            #[cfg(feature = "http3")]
            Self::H3(body) => BoxBody::new(body),
            Self::Boxed(body) => body,
            Self::None => BoxBody::new(Empty::<Bytes>::new()),
        }
    }
}

impl Body for RequestBody {
    type Data = Bytes;
    type Error = BodyError;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            #[cfg(feature = "http2")]
            Self::H2(body) => Pin::new(body).poll_frame(cx),
            #[cfg(feature = "http3")]
            Self::H3(body) => Pin::new(body).poll_frame(cx),
            Self::Boxed(body) => Pin::new(body).poll_frame(cx),
            Self::None => Poll::Ready(None),
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        match self {
            #[cfg(feature = "http2")]
            Self::H2(body) => body.is_end_stream(),
            #[cfg(feature = "http3")]
            Self::H3(body) => body.is_end_stream(),
            Self::Boxed(body) => body.is_end_stream(),
            Self::None => true,
        }
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        match self {
            #[cfg(feature = "http2")]
            Self::H2(body) => body.size_hint(),
            #[cfg(feature = "http3")]
            Self::H3(body) => body.size_hint(),
            Self::Boxed(body) => body.size_hint(),
            Self::None => SizeHint::None,
        }
    }
}

impl From<Bytes> for RequestBody {
    fn from(bytes: Bytes) -> Self {
        Self::from(BoxBody::new(Full::new(bytes)))
    }
}

impl From<BoxBody> for RequestBody {
    fn from(body: BoxBody) -> Self {
        Self::Boxed(body)
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

/// type erased body. This is a `!Send` box, unlike `http_body_util::UnsyncBoxBody`.
pub struct BoxBody(Pin<Box<dyn Body<Data = Bytes, Error = BodyError>>>);

impl Default for BoxBody {
    fn default() -> Self {
        Self::new(Empty::<Bytes>::new())
    }
}

impl BoxBody {
    #[inline]
    pub fn new<B, T, E>(body: B) -> Self
    where
        B: Body<Data = T, Error = E> + 'static,
        T: Into<Bytes> + Buf + 'static,
        E: Into<BodyError> + 'static,
    {
        pin_project! {
            struct MapBody<B> {
                #[pin]
                body: B
            }
        }

        impl<B, T, E> Body for MapBody<B>
        where
            B: Body<Data = T, Error = E>,
            T: Into<Bytes> + Buf,
            E: Into<BodyError>,
        {
            type Data = Bytes;
            type Error = BodyError;

            #[inline]
            fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
                self.project()
                    .body
                    .poll_frame(cx)
                    .map_ok(|frame| match frame {
                        Frame::Data(data) => Frame::Data(data.into()),
                        Frame::Trailers(trailers) => Frame::Trailers(trailers),
                    })
                    .map_err(Into::into)
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

        Self(Box::pin(MapBody { body }))
    }
}

impl Body for BoxBody {
    type Data = Bytes;
    type Error = BodyError;

    #[inline]
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        self.get_mut().0.as_mut().poll_frame(cx)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }
}

pin_project! {
    /// A unified response body type.
    /// Generic type is for custom pinned response body(type implement [Body](http_body::Body)).
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
        Body {
            #[pin]
            stream: B,
        },
    }
}

impl<B> Default for ResponseBody<B> {
    fn default() -> Self {
        Self::empty()
    }
}

impl ResponseBody {
    /// Construct a new Body variant of ResponseBody with default type as [BoxBody]
    #[inline]
    pub fn boxed<B, T, E>(body: B) -> Self
    where
        B: Body<Data = T, Error = E> + 'static,
        T: Into<Bytes> + Buf + 'static,
        E: Into<BodyError> + 'static,
    {
        Self::body(BoxBody::new(body))
    }
}

impl<B> ResponseBody<B> {
    /// indicate empty body is attached to response.
    /// `content-length: 0` header would be added to response when [BodySize] is
    /// used for inferring response body type.
    #[inline]
    pub const fn empty() -> Self {
        Self {
            inner: ResponseBodyInner::None,
        }
    }

    /// Construct a new Body variant of ResponseBody
    #[inline]
    pub const fn body(stream: B) -> Self {
        Self {
            inner: ResponseBodyInner::Body { stream },
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
    pub fn into_boxed<T, E>(self) -> ResponseBody
    where
        B: Body<Data = T, Error = E> + 'static,
        T: Into<Bytes> + Buf + 'static,
        E: error::Error + Send + Sync + 'static,
    {
        match self.inner {
            ResponseBodyInner::None => ResponseBody::empty(),
            ResponseBodyInner::Bytes { bytes } => ResponseBody::bytes(bytes),
            ResponseBodyInner::Body { stream } => ResponseBody::boxed(stream),
        }
    }
}

impl<B, E> Body for ResponseBody<B>
where
    B: Body<Data = Bytes, Error = E>,
{
    type Data = Bytes;
    type Error = E;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, E>>> {
        let mut inner = self.project().inner;
        match inner.as_mut().project() {
            ResponseBodyProj::None => Poll::Ready(None),
            ResponseBodyProj::Bytes { .. } => match inner.project_replace(ResponseBodyInner::None) {
                ResponseBodyProjReplace::Bytes { bytes } => Poll::Ready(Some(Ok(Frame::Data(bytes)))),
                _ => unreachable!(),
            },
            ResponseBodyProj::Body { stream } => stream.poll_frame(cx),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self.inner {
            ResponseBodyInner::None => true,
            // see poll_frame method for reason. bytes variant always yield once on poll
            ResponseBodyInner::Bytes { .. } => false,
            ResponseBodyInner::Body { ref stream } => stream.is_end_stream(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self.inner {
            ResponseBodyInner::None => SizeHint::None,
            ResponseBodyInner::Bytes { ref bytes } => SizeHint::Exact(bytes.len() as u64),
            ResponseBodyInner::Body { ref stream } => stream.size_hint(),
        }
    }
}

impl From<BoxBody> for ResponseBody {
    fn from(body: BoxBody) -> Self {
        Self::boxed(body)
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
