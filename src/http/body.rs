use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::stream::{LocalBoxStream, Stream};
use pin_project_lite::pin_project;

use super::error::BodyError;

/// A unified request body type for different http protocols.
/// This enables one service type to handle multiple http protocols.
pub enum RequestBody {
    H2(super::h2::RequestBody),
}

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::H2(body) => Pin::new(body).poll_next(cx),
        }
    }
}

pub type StreamBody = LocalBoxStream<'static, Result<Bytes, BodyError>>;

pin_project! {
    /// A unified response body type.
    /// Generic type is for custom pinned response body.
    #[project = ResponseBodyProj]
    #[project_replace = ResponseBodyProjReplace]
    pub enum ResponseBody<B = StreamBody> {
        None,
        Bytes {
            bytes: Bytes
        },
        Stream {
            #[pin]
            stream: B
        },
    }
}

impl<B, E> ResponseBody<B>
where
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    pub fn is_eof(&self) -> bool {
        match *self {
            Self::None => true,
            Self::Bytes { ref bytes } => bytes.is_empty(),
            Self::Stream { .. } => false,
        }
    }

    /// Construct a new Stream variant of ResponseBody
    pub fn stream(stream: B) -> Self {
        Self::Stream { stream }
    }
}

impl<B, E> Stream for ResponseBody<B>
where
    // TODO: Make error generic.
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    type Item = Result<Bytes, BodyError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.as_mut().project() {
            ResponseBodyProj::None => Poll::Ready(None),
            ResponseBodyProj::Bytes { .. } => match self.project_replace(ResponseBody::None) {
                ResponseBodyProjReplace::Bytes { bytes } => Poll::Ready(Some(Ok(bytes))),
                _ => unreachable!(),
            },
            ResponseBodyProj::Stream { stream } => stream.poll_next(cx).map_err(From::from),
        }
    }
}

impl<B> From<Bytes> for ResponseBody<B> {
    fn from(bytes: Bytes) -> Self {
        Self::Bytes { bytes }
    }
}

impl From<StreamBody> for ResponseBody {
    fn from(stream: StreamBody) -> Self {
        Self::Stream { stream }
    }
}
