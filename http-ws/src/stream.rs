use std::{
    fmt,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use pin_project_lite::pin_project;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use super::codec::{Codec, Message};
use super::error::ProtocolError;

pin_project! {
    /// Decode `S` type into Stream of websocket [Message](super::codec::Message).
    /// `S` type must impl `Stream` trait and output `Result<T, E>` as `Stream::Item`
    /// where `T` type impl `AsRef<[u8]>` trait. (`&[u8]` is needed for parsing messages)
    pub struct DecodeStream<S> {
        #[pin]
        stream: Option<S>,
        buf: BytesMut,
        codec: Rc<Codec>
    }
}

impl<S, T, E> DecodeStream<S>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    #[inline]
    pub fn new(stream: S) -> Self {
        Self::with_codec(stream, Codec::new())
    }

    pub fn with_codec(stream: S, codec: Codec) -> Self {
        Self {
            stream: Some(stream),
            buf: BytesMut::new(),
            codec: Rc::new(codec),
        }
    }

    /// Make an [EncodeStream] from current DecodeStream.
    ///
    /// This API is to share the same codec for both decode and encode stream.
    pub fn encode_stream(&self) -> (Sender<Message>, EncodeStream) {
        let codec = self.codec.clone();
        EncodeStream::new(codec)
    }

    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&mut self) -> Next<'_, Self> {
        Next { stream: self }
    }
}

pub enum DecodeError<E> {
    Protocol(ProtocolError),
    Stream(E),
}

impl<E> fmt::Debug for DecodeError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Protocol(ref e) => write!(f, "{:?}", e),
            Self::Stream(..) => write!(f, "Input Stream error"),
        }
    }
}

impl<E> fmt::Display for DecodeError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Protocol(ref e) => write!(f, "{:?}", e),
            Self::Stream(..) => write!(f, "Input Stream error"),
        }
    }
}

impl<E> std::error::Error for DecodeError<E> {}

impl<E> From<ProtocolError> for DecodeError<E> {
    fn from(e: ProtocolError) -> Self {
        Self::Protocol(e)
    }
}

impl<S, T, E> Stream for DecodeStream<S>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    type Item = Result<Message, DecodeError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        while let Some(stream) = this.stream.as_mut().as_pin_mut() {
            match stream.poll_next(cx) {
                Poll::Ready(Some(Ok(item))) => {
                    this.buf.extend_from_slice(item.as_ref());
                    if this.buf.len() >= this.codec.get_max_size() {
                        break;
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(DecodeError::Stream(e)))),
                Poll::Ready(None) => this.stream.set(None),
                Poll::Pending => break,
            }
        }

        match this.codec.decode(this.buf)? {
            Some(msg) => Poll::Ready(Some(Ok(msg))),
            None => {
                if this.stream.is_none() {
                    Poll::Ready(None)
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

/// Encode a stream of [Message](super::codec::Message) into [Bytes](bytes::Bytes).
pub struct EncodeStream {
    codec: Rc<Codec>,
    buf: BytesMut,
    rx: Option<Receiver<Message>>,
}

impl EncodeStream {
    /// Construct new stream with given codec.
    #[inline]
    pub fn new(codec: Rc<Codec>) -> (Sender<Message>, Self) {
        let cap = codec.capacity();
        let (tx, rx) = channel(cap);

        let stream = EncodeStream {
            codec,
            buf: BytesMut::new(),
            rx: Some(rx),
        };

        (tx, stream)
    }

    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&mut self) -> Next<'_, Self> {
        Next { stream: self }
    }
}

impl Stream for EncodeStream {
    type Item = Result<Bytes, ProtocolError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        while let Some(rx) = this.rx.as_mut() {
            match rx.poll_recv(cx) {
                Poll::Ready(Some(msg)) => this.codec.encode(msg, &mut this.buf)?,
                Poll::Ready(None) => this.rx = None,
                Poll::Pending => break,
            }
        }

        if !this.buf.is_empty() {
            Poll::Ready(Some(Ok(this.buf.split().freeze())))
        } else if this.rx.is_none() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

pin_project! {
    pub struct Next<'a, S> {
        #[pin]
        stream: &'a mut S
    }
}

impl<S> Future for Next<'_, S>
where
    S: Stream + Unpin,
{
    type Output = Option<S::Item>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().stream).poll_next(cx)
    }
}
