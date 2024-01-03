use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
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

    /// add [io::Error] to [ResponseStream].
    ///
    /// the error should be used as a signal to the TCP connection associated with `ResponseStream`
    /// to close immediately.
    ///
    /// # Examples
    /// ```rust
    /// use std::{future::poll_fn, pin::Pin, time::Duration};
    ///
    /// use futures_core::Stream;
    /// use http_ws::{CloseCode, Message, RequestStream, ResponseSender, ResponseStream};
    /// use tokio::{io::AsyncWriteExt, time::timeout, net::TcpStream};
    ///
    /// // thread1:
    /// // read and write websocket message.
    /// async fn sender<S, T, E>(tx: ResponseSender, mut rx: Pin<&mut RequestStream<S, E>>)
    /// where
    ///     S: Stream<Item = Result<T, E>>,
    ///     T: AsRef<[u8]>,
    /// {
    ///     // send close message to client
    ///     tx.send(Message::Close(Some(CloseCode::Away.into()))).await.unwrap();
    ///
    ///     // the client failed to respond to close message in 5 seconds time window.
    ///     if let Err(_) = timeout(Duration::from_secs(5), poll_fn(|cx| rx.as_mut().poll_next(cx))).await {
    ///         // send io error to thread2
    ///         tx.send_error(std::io::ErrorKind::UnexpectedEof.into()).await.unwrap();
    ///     }
    /// }
    ///
    /// // thread2:
    /// // receive websocket message from thread1 and transfer it on tcp connection.
    /// async fn io_write(conn: &mut TcpStream, mut rx: Pin<&mut ResponseStream>) {
    ///     // the first message is the "go away" close message in Ok branch.
    ///     let msg = poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().unwrap();
    ///
    ///     // send msg to client
    ///     conn.write_all(&msg).await.unwrap();
    ///
    ///     // the second message is the io::Error in Err branch.
    ///     let err = poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().unwrap_err();
    ///
    ///     // at this point we should close the tcp connection by either graceful close or
    ///     // just return immediately and drop the TcpStream.
    ///     let _ = conn.shutdown().await;
    /// }
    ///
    /// // thread3:
    /// // receive message from tcp connection and send it to thread1:
    /// async fn io_read(conn: &mut TcpStream) {
    ///     // this part is ignored as it has no relation to the send_error api.
    /// }
    /// ```
    #[inline]
    pub fn send_error(&self, err: io::Error) -> impl Future<Output = Result<(), ProtocolError>> + '_ {
        self.inner.send_error(err)
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
    tx: Sender<Item>,
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

    // send message to response stream. it would produce Ok(bytes) when succeed where
    // the bytes is encoded binary websocket message ready to be sent to client.
    async fn send(&self, msg: Message) -> Result<(), ProtocolError> {
        let buf = self.encode(msg)?;
        self.tx.send(Ok(buf)).await.map_err(|_| ProtocolError::Closed)
    }

    // send error to response stream. it would produce Err(io::Error) when succeed where
    // the error is a representation of io error to the stream consumer. in most cases
    // the consumer observing the error should close the stream and the tcp connection
    // the stream belongs to.
    async fn send_error(&self, err: io::Error) -> Result<(), ProtocolError> {
        self.tx.send(Err(err)).await.map_err(|_| ProtocolError::Closed)
    }
}
