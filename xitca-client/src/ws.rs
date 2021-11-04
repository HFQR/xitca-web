use std::{
    cell::RefCell,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::{ready, stream::Stream};
use futures_util::sink::Sink;
use http_ws::{Codec, Message};
use xitca_http::bytes::{Buf, BytesMut};
use xitca_io::io::AsyncWrite;

use super::{connection::ConnectionWithKey, error::Error, h1};

/// sender part of websocket connection.
/// [Sink] trait is used to asynchronously send message.
pub struct WebSocketSink<'a, 'b>(&'a WebSocket<'b>);

impl Sink<Message> for WebSocketSink<'_, '_> {
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut *self.get_mut().0.inner.borrow_mut()).poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        Pin::new(&mut *self.get_mut().0.inner.borrow_mut()).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut *self.get_mut().0.inner.borrow_mut()).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut *self.get_mut().0.inner.borrow_mut()).poll_close(cx)
    }
}

/// sender part of websocket connection.
/// [Stream] trait is used to asynchronously receive message.
pub struct WebSocketReader<'a, 'b>(&'a WebSocket<'b>);

impl Stream for WebSocketReader<'_, '_> {
    type Item = Result<Message, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut *self.get_mut().0.inner.borrow_mut()).poll_next(cx)
    }
}

/// A unified websocket that can be used as both sender/receiver.
///
/// * This type can not do concurrent message handling which means send always block receive
/// and vice versa.
pub struct WebSocket<'c> {
    inner: RefCell<WebSocketInner<'c>>,
}

impl<'a> WebSocket<'a> {
    pub(crate) fn from_body(body: h1::body::ResponseBody<ConnectionWithKey<'a>>) -> Self {
        Self {
            inner: RefCell::new(WebSocketInner {
                codec: Codec::new().client_mode(),
                send_buf: BytesMut::new(),
                recv_buf: BytesMut::new(),
                body,
            }),
        }
    }

    /// Split into a sink and reader pair that can be used for concurrent read/write
    /// message to websocket connection.
    #[inline]
    pub fn split<'s>(&'s self) -> (WebSocketSink<'s, 'a>, WebSocketReader<'s, 'a>) {
        (WebSocketSink(&*self), WebSocketReader(&*self))
    }

    /// Set max message size.
    ///
    /// By default max size is set to 64kB.
    pub fn max_size(self, size: usize) -> Self {
        let WebSocketInner {
            codec,
            send_buf,
            recv_buf,
            body,
        } = self.inner.into_inner();

        Self {
            inner: RefCell::new(WebSocketInner {
                codec: codec.max_size(size),
                send_buf,
                recv_buf,
                body,
            }),
        }
    }
}

impl Sink<Message> for WebSocket<'_> {
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(self.get_mut().inner.get_mut()).poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        Pin::new(self.get_mut().inner.get_mut()).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(self.get_mut().inner.get_mut()).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(self.get_mut().inner.get_mut()).poll_close(cx)
    }
}

impl Stream for WebSocket<'_> {
    type Item = Result<Message, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(self.get_mut().inner.get_mut()).poll_next(cx)
    }
}

struct WebSocketInner<'c> {
    codec: Codec,
    send_buf: BytesMut,
    recv_buf: BytesMut,
    body: h1::body::ResponseBody<ConnectionWithKey<'c>>,
}

impl Sink<Message> for WebSocketInner<'_> {
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // TODO: set up a meaningful backpressure limit for send buf.
        if !self.as_mut().get_mut().send_buf.chunk().is_empty() {
            self.poll_flush(cx)
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let inner = self.get_mut();

        inner.codec.encode(item, &mut inner.send_buf).map_err(Into::into)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let inner = self.get_mut();

        while !inner.send_buf.chunk().is_empty() {
            match ready!(Pin::new(&mut **inner.body.conn()).poll_write(cx, inner.send_buf.chunk()))? {
                0 => return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())),
                n => inner.send_buf.advance(n),
            }
        }

        Pin::new(&mut **inner.body.conn()).poll_flush(cx).map_err(Into::into)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.as_mut().poll_flush(cx))?;
        Pin::new(&mut **self.get_mut().body.conn())
            .poll_shutdown(cx)
            .map_err(Into::into)
    }
}

impl Stream for WebSocketInner<'_> {
    type Item = Result<Message, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        while let Some(res) = ready!(Pin::new(&mut this.body).poll_next(cx)) {
            let bytes = res?;
            this.recv_buf.extend_from_slice(&bytes);

            if let Some(msg) = this.codec.decode(&mut this.recv_buf)? {
                return Poll::Ready(Some(Ok(msg)));
            }
        }

        Poll::Ready(None)
    }
}
