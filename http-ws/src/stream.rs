use core::{
    convert::Infallible,
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use alloc::sync::{Arc, Weak};

use std::sync::Mutex;

use std::error;

use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use super::{
    codec::{Codec, Message},
    error::ProtocolError,
};

pin_project! {
    /// Decode `S` type into Stream of websocket [Message].
    /// `S` type must impl `Stream` trait and output `Result<T, E>` as `Stream::Item`
    /// where `T` type impl `AsRef<[u8]>` trait. (`&[u8]` is needed for parsing messages)
    pub struct RequestStream<S, E> {
        #[pin]
        stream: Option<S>,
        buf: BytesMut,
        codec: Codec,
        err: Option<WsError<E>>
    }
}

impl<S, T, E> RequestStream<S, E>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    pub fn new(stream: S) -> Self {
        Self::with_codec(stream, Codec::new())
    }

    pub fn with_codec(stream: S, codec: Codec) -> Self {
        Self {
            stream: Some(stream),
            buf: BytesMut::new(),
            codec,
            err: None,
        }
    }

    /// Make a [ResponseStream] from current DecodeStream.
    ///
    /// This API is to share the same codec for both decode and encode stream.
    pub fn response_stream(&self) -> (ResponseStream, ResponseSender) {
        let codec = self.codec.duplicate();
        let cap = codec.capacity();
        let (tx, rx) = channel(cap);
        (ResponseStream(rx), ResponseSender::new(tx, codec))
    }
}

pub enum WsError<E> {
    Protocol(ProtocolError),
    Stream(E),
}

impl<E> fmt::Debug for WsError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Protocol(ref e) => fmt::Debug::fmt(e, f),
            Self::Stream(..) => f.write_str("Input Stream error"),
        }
    }
}

impl<E> fmt::Display for WsError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Protocol(ref e) => fmt::Debug::fmt(e, f),
            Self::Stream(..) => f.write_str("Input Stream error"),
        }
    }
}

impl<E> error::Error for WsError<E> {}

impl<E> From<ProtocolError> for WsError<E> {
    fn from(e: ProtocolError) -> Self {
        Self::Protocol(e)
    }
}

impl<S, T, E> Stream for RequestStream<S, E>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    type Item = Result<Message, WsError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        while let Some(stream) = this.stream.as_mut().as_pin_mut() {
            match stream.poll_next(cx) {
                Poll::Ready(Some(Ok(item))) => {
                    this.buf.extend_from_slice(item.as_ref());
                    if this.buf.len() > this.codec.max_size() {
                        break;
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    *this.err = Some(WsError::Stream(e));
                    this.stream.set(None);
                }
                Poll::Ready(None) => this.stream.set(None),
                Poll::Pending => break,
            }
        }

        match this.codec.decode(this.buf)? {
            Some(msg) => Poll::Ready(Some(Ok(msg))),
            None => {
                if this.stream.is_none() {
                    Poll::Ready(this.err.take().map(Err))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

pub struct ResponseStream(Receiver<Bytes>);

impl Stream for ResponseStream {
    type Item = Result<Bytes, Infallible>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().0.poll_recv(cx).map(|res| res.map(Ok))
    }
}

/// Encode [Message] into [Bytes] and send it to [ResponseStream].
#[derive(Debug)]
pub struct ResponseSender {
    inner: Arc<_ResponseSender>,
}

impl ResponseSender {
    fn new(tx: Sender<Bytes>, codec: Codec) -> Self {
        Self {
            inner: Arc::new(_ResponseSender {
                encoder: Mutex::new(Encoder {
                    codec,
                    buf: BytesMut::with_capacity(codec.max_size()),
                }),
                tx,
            }),
        }
    }

    /// downgrade Self to a weak sender.
    pub fn downgrade(&self) -> ResponseWeakSender {
        ResponseWeakSender {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// encode [Message] and add to [ResponseStream].
    #[inline]
    pub fn send(&self, msg: Message) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.inner.send(msg)
    }

    /// encode [Message::Text] variant and add to [ResponseStream].
    #[inline]
    pub fn text(&self, txt: impl Into<String>) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.send(Message::Text(Bytes::from(txt.into())))
    }

    /// encode [Message::Binary] variant and add to [ResponseStream].
    #[inline]
    pub fn binary(&self, bin: impl Into<Bytes>) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.send(Message::Binary(bin.into()))
    }
}

/// [Weak] version of [ResponseSender].
#[derive(Debug)]
pub struct ResponseWeakSender {
    inner: Weak<_ResponseSender>,
}

impl ResponseWeakSender {
    /// upgrade self to strong response sender.
    /// return None when [ResponseSender] is already dropped.
    pub fn upgrade(&self) -> Option<ResponseSender> {
        self.inner.upgrade().map(|inner| ResponseSender { inner })
    }
}

#[derive(Debug)]
struct _ResponseSender {
    encoder: Mutex<Encoder>,
    tx: Sender<Bytes>,
}

#[derive(Debug)]
struct Encoder {
    codec: Codec,
    buf: BytesMut,
}

impl _ResponseSender {
    fn encode(&self, msg: Message) -> Result<Bytes, ProtocolError> {
        let mut encoder = self.encoder.lock().unwrap();
        let Encoder {
            ref mut codec,
            ref mut buf,
        } = *encoder;
        codec.encode(msg, buf).map(|_| buf.split().freeze())
    }

    async fn send(&self, msg: Message) -> Result<(), ProtocolError> {
        let buf = self.encode(msg)?;
        self.tx.send(buf).await.map_err(|_| ProtocolError::Closed)
    }
}
