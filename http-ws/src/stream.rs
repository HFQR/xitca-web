use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use alloc::sync::{Arc, Weak};

use std::{error, io, sync::Mutex};

use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use super::{
    codec::{Codec, Message},
    error::ProtocolError,
    proto::CloseReason,
};

pin_project! {
    /// Decode `S` type into Stream of websocket [Message].
    /// `S` type must impl `Stream` trait and output `Result<T, E>` as `Stream::Item`
    /// where `T` type impl `AsRef<[u8]>` trait. (`&[u8]` is needed for parsing messages)
    ///
    /// # Stream termination
    ///
    /// This stream never returns `None`. Callers should expect one of the following outcomes:
    ///
    /// - `Ok(Message::Close(_))`: The remote peer initiated a clean close. The caller should
    ///   send a close frame back via [ResponseSender] and then stop polling.
    /// - `Err(WsError::Protocol(ProtocolError::RecvClosed))`: The stream was polled after a
    ///   close frame had already been received. The caller should have stopped polling after
    ///   observing `Message::Close`.
    /// - `Err(WsError::Protocol(ProtocolError::UnexpectedEof))`: The underlying transport
    ///   ended without a close frame. This is an abnormal closure and the associated
    ///   connection should not be reused.
    /// - `Err(WsError::Protocol(_))`: A protocol violation occurred (e.g. bad opcode,
    ///   continuation error). The connection should be dropped.
    /// - `Err(WsError::Stream(_))`: The underlying stream produced an error.
    pub struct RequestStream<S> {
        #[pin]
        stream: S,
        buf: BytesMut,
        codec: Codec,
    }
}

impl<S, T, E> RequestStream<S>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    pub fn new(stream: S) -> Self {
        Self::with_codec(stream, Codec::new())
    }

    pub fn with_codec(stream: S, codec: Codec) -> Self {
        Self {
            stream,
            buf: BytesMut::new(),
            codec,
        }
    }

    #[inline]
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    #[inline]
    pub fn codec_mut(&mut self) -> &mut Codec {
        &mut self.codec
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

impl<S, T, E> Stream for RequestStream<S>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    type Item = Result<Message, WsError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match this.codec.decode(this.buf)? {
                Some(msg) => return Poll::Ready(Some(Ok(msg))),
                None => match ready!(this.stream.as_mut().poll_next(cx)) {
                    Some(res) => {
                        let item = res.map_err(WsError::Stream)?;
                        this.buf.extend_from_slice(item.as_ref())
                    }
                    None => return Poll::Ready(Some(Err(WsError::Protocol(ProtocolError::UnexpectedEof)))),
                },
            }
        }
    }
}

pub struct ResponseStream(Receiver<Item>);

type Item = io::Result<Bytes>;

impl Stream for ResponseStream {
    type Item = Item;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().0.poll_recv(cx)
    }
}

/// Encode [Message] into [Bytes] and send it to [ResponseStream].
#[derive(Debug)]
pub struct ResponseSender {
    inner: Arc<_ResponseSender>,
}

impl ResponseSender {
    fn new(tx: Sender<Item>, codec: Codec) -> Self {
        let buf = BytesMut::with_capacity(codec.max_size());
        Self {
            inner: Arc::new(_ResponseSender {
                encoder: Mutex::new(Encoder { codec, buf }),
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

    /// add [io::Error] to [ResponseStream].
    ///
    /// the error should be used as a signal to the TCP connection associated with `ResponseStream`
    /// to close immediately.
    #[inline]
    pub fn send_error(&self, err: io::Error) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.inner.send_error(err)
    }

    /// encode [Message::Text] variant and add to [ResponseStream].
    #[inline]
    pub async fn text(&self, txt: impl Into<Bytes>) -> Result<(), ProtocolError> {
        let bytes = txt.into();
        core::str::from_utf8(&bytes).map_err(|_| ProtocolError::BadOpCode)?;
        self.send(Message::Text(bytes)).await
    }

    /// encode [Message::Binary] variant and add to [ResponseStream].
    #[inline]
    pub fn binary(&self, bin: impl Into<Bytes>) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.send(Message::Binary(bin.into()))
    }

    /// encode [Message::Continuation] variant and add to [ResponseStream].
    #[inline]
    pub fn continuation(&self, item: super::codec::Item) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.send(Message::Continuation(item))
    }

    /// encode [Message::Ping] variant and add to [ResponseStream].
    #[inline]
    pub fn ping(&self, bin: impl Into<Bytes>) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.send(Message::Ping(bin.into()))
    }

    /// encode [Message::Close] variant and add to [ResponseStream].
    /// take ownership of Self as after close message no more message can be sent to client.
    pub async fn close(self, reason: Option<impl Into<CloseReason>>) -> Result<(), ProtocolError> {
        self.send(Message::Close(reason.map(Into::into))).await
    }

    #[inline]
    fn send(&self, msg: Message) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.inner.send(msg)
    }
}

/// [Weak] version of [ResponseSender].
#[derive(Debug)]
pub struct ResponseWeakSender {
    inner: Weak<_ResponseSender>,
}

impl ResponseWeakSender {
    /// upgrade self to strong response sender.
    /// return None when [ResponseSender] is already dropped or session is already closed
    pub fn upgrade(&self) -> Option<ResponseSender> {
        self.inner.upgrade().and_then(|inner| {
            let closed = inner.encoder.lock().unwrap().codec.send_closed();
            (!closed).then(|| ResponseSender { inner })
        })
    }
}

#[derive(Debug)]
struct _ResponseSender {
    encoder: Mutex<Encoder>,
    tx: Sender<Item>,
}

#[derive(Debug)]
struct Encoder {
    codec: Codec,
    buf: BytesMut,
}

impl _ResponseSender {
    // send message to response stream. it would produce Ok(bytes) when succeed where
    // the bytes is encoded binary websocket message ready to be sent to client.
    async fn send(&self, msg: Message) -> Result<(), ProtocolError> {
        let permit = self.tx.reserve().await.map_err(|_| ProtocolError::SendClosed)?;
        let buf = {
            let mut encoder = self.encoder.lock().unwrap();
            let Encoder { codec, buf } = &mut *encoder;
            codec.encode(msg, buf)?;
            buf.split().freeze()
        };
        permit.send(Ok(buf));
        Ok(())
    }

    // send error to response stream. it would produce Err(io::Error) when succeed where
    // the error is a representation of io error to the stream consumer. in most cases
    // the consumer observing the error should close the stream and the tcp connection
    // the stream belongs to.
    async fn send_error(&self, err: io::Error) -> Result<(), ProtocolError> {
        self.tx.send(Err(err)).await.map_err(|_| ProtocolError::SendClosed)
    }
}
