pub(crate) use xitca_http::{
    body::{self, ResponseBodySize},
    bytes::Bytes,
    error::BodyError,
};

use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::{BoxStream, Stream};

use crate::{connection::ConnectionWithKey, h1};

/// Default stream body type are boxed stream trait object that is `Send`.
pub type StreamBody = BoxStream<'static, Result<Bytes, BodyError>>;

/// When used by client [body::ResponseBody] is used as Request body.
pub type RequestBody<B = StreamBody> = body::ResponseBody<B>;

/// When used by client [ResponseBodySize] is used as Request body size.
pub type RequestBodySize = ResponseBodySize;

#[allow(clippy::large_enum_variant)]
pub enum ResponseBody<'c> {
    H1(h1::body::ResponseBody<ConnectionWithKey<'c>>),
    #[cfg(feature = "http2")]
    H2(crate::h2::body::ResponseBody),
    #[cfg(feature = "http3")]
    H3(crate::h3::body::ResponseBody),
    // TODO: add http1 eof resposne body variant.
    #[allow(dead_code)]
    Eof,
}

impl ResponseBody<'_> {
    pub(crate) fn destroy_on_drop(&mut self) {
        if let Self::H1(ref mut body) = *self {
            body.conn().destroy_on_drop()
        }
    }
}

impl fmt::Debug for ResponseBody<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::H1(_) => write!(f, "ResponseBody::H1(..)"),
            #[cfg(feature = "http2")]
            Self::H2(_) => write!(f, "ResponseBody::H2(..)"),
            #[cfg(feature = "http3")]
            Self::H3(_) => write!(f, "ResponseBody::H3(..)"),
            Self::Eof => write!(f, "ResponseBody::Eof"),
        }
    }
}

impl Stream for ResponseBody<'_> {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::H1(body) => Pin::new(body).poll_next(cx),
            #[cfg(feature = "http2")]
            Self::H2(body) => Pin::new(body).poll_next(cx),
            #[cfg(feature = "http3")]
            Self::H3(body) => Pin::new(body).poll_next(cx),
            Self::Eof => Poll::Ready(None),
        }
    }
}
