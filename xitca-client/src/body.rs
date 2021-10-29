pub(crate) use xitca_http::{
    body::{self, ResponseBodySize},
    bytes::Bytes,
    error::BodyError,
};

use std::{
    fmt, io,
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

pub enum ResponseBody<'c> {
    H1(h1::body::ResponseBody<ConnectionWithKey<'c>>),
}

impl fmt::Debug for ResponseBody<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::H1(_) => write!(f, "ResponseBody::H1(..)"),
        }
    }
}

impl Stream for ResponseBody<'_> {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::H1(body) => Pin::new(body).poll_next(cx),
        }
    }
}
