pub(crate) use xitca_http::{
    body::{Body, BodyExt, Data, Empty, Frame, SizeHint, Trailers},
    error::BodyError,
};

use core::{
    any::Any,
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;

use crate::bytes::Bytes;

/// body type for http request
pub type RequestBody = xitca_http::body::ResponseBody<BoxBody>;

// conditional boxing body to convert generic body type to conrete
pub(crate) fn downcast_body<B>(body: B) -> RequestBody
where
    B: Body + Send + 'static,
    B::Data: Into<Bytes>,
    B::Error: Into<BodyError>,
{
    let body = &mut Some(body);
    match (body as &mut dyn Any).downcast_mut::<Option<RequestBody>>() {
        Some(body) => body.take().unwrap(),
        None => RequestBody::body(BoxBody::new(body.take().unwrap())),
    }
}

#[allow(clippy::large_enum_variant)]
pub enum ResponseBody {
    #[cfg(feature = "http1")]
    H1(crate::h1::body::ResponseBody),
    #[cfg(feature = "http2")]
    H2(crate::h2::body::ResponseBody),
    #[cfg(feature = "http3")]
    H3(crate::h3::body::ResponseBody),
    Eof,
    Unknown(Pin<Box<dyn Body<Data = Bytes, Error = BodyError> + Send + Sync + 'static>>),
}

impl fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            #[cfg(feature = "http1")]
            Self::H1(_) => f.write_str("ResponseBody::H1(..)"),
            #[cfg(feature = "http2")]
            Self::H2(_) => f.write_str("ResponseBody::H2(..)"),
            #[cfg(feature = "http3")]
            Self::H3(_) => f.write_str("ResponseBody::H3(..)"),
            Self::Eof => f.write_str("ResponseBody::Eof"),
            Self::Unknown(_) => f.write_str("ResponseBody::Unknown"),
        }
    }
}

impl ResponseBody {
    pub(crate) fn destroy_on_drop(&mut self) {
        #[cfg(feature = "http1")]
        if let Self::H1(ref mut body) = *self {
            body.conn_mut().mark_destroy()
        }
    }

    pub(crate) fn can_destroy_on_drop(&mut self) -> bool {
        #[cfg(feature = "http1")]
        if let Self::H1(ref mut body) = *self {
            return body.conn_mut().is_marked_destroy();
        }

        false
    }
}

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        match self.get_mut() {
            #[cfg(feature = "http1")]
            Self::H1(body) => Pin::new(body).poll_frame(cx),
            #[cfg(feature = "http2")]
            Self::H2(body) => Pin::new(body).poll_frame(cx),
            #[cfg(feature = "http3")]
            Self::H3(body) => Pin::new(body).poll_frame(cx),
            Self::Eof => Poll::Ready(None),
            Self::Unknown(body) => body.as_mut().poll_frame(cx),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            #[cfg(feature = "http1")]
            Self::H1(body) => body.is_end_stream(),
            #[cfg(feature = "http2")]
            Self::H2(body) => body.is_end_stream(),
            #[cfg(feature = "http3")]
            Self::H3(body) => body.is_end_stream(),
            Self::Eof => true,
            Self::Unknown(body) => body.is_end_stream(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            #[cfg(feature = "http1")]
            Self::H1(body) => body.size_hint(),
            #[cfg(feature = "http2")]
            Self::H2(body) => body.size_hint(),
            #[cfg(feature = "http3")]
            Self::H3(body) => body.size_hint(),
            Self::Eof => SizeHint::None,
            Self::Unknown(body) => body.size_hint(),
        }
    }
}

// Stream impl kept for compatibility with http-ws crate's RequestStream.
impl Stream for ResponseBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.poll_frame(cx).map(|opt| {
            opt.and_then(|res| match res {
                Ok(Frame::Data(data)) => Some(Ok(data)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match Body::size_hint(self) {
            SizeHint::Exact(len) => {
                let len = usize::try_from(len).unwrap();
                (len, Some(len))
            }
            SizeHint::Unknown => (0, None),
            SizeHint::None => SizeHint::NO_BODY_HINT,
        }
    }
}

/// type erased body.
pub struct BoxBody(Pin<Box<dyn Body<Data = Bytes, Error = BodyError> + Send + 'static>>);

impl Default for BoxBody {
    fn default() -> Self {
        Self::new(Empty::<Bytes>::new())
    }
}

impl BoxBody {
    #[inline]
    pub fn new<B>(body: B) -> Self
    where
        B: Body + Send + 'static,
        B::Data: Into<Bytes>,
        B::Error: Into<BodyError>,
    {
        Self(Box::pin(BoxBodyMap { body }))
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
    struct BoxBodyMap<B> {
        #[pin]
        body: B
    }
}

impl<B> Body for BoxBodyMap<B>
where
    B: Body,
    B::Data: Into<Bytes>,
    B::Error: Into<BodyError>,
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
