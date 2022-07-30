pub(crate) use xitca_http::{
    body::{BodySize, NoneBody, Once},
    error::BodyError,
};

use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::Stream;

use crate::{bytes::Bytes, connection::ConnectionWithKey, h1};

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

    pub(crate) fn can_destroy_on_drop(&mut self) -> bool {
        if let Self::H1(ref mut body) = *self {
            body.conn().is_destroy_on_drop()
        } else {
            false
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
