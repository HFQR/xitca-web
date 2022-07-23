pub use http_ws::Message;

use std::{
    cell::RefCell,
    io,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use futures_util::sink::Sink;
use http_ws::Codec;
use xitca_http::bytes::{Buf, BytesMut};
use xitca_io::io::AsyncWrite;

use super::{body::ResponseBody, error::Error};

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
    pub(crate) fn try_from_body(body: ResponseBody<'a>) -> Result<Self, Error> {
        // TODO: check body to make sure only H1 and H2 are accepted.

        Ok(Self {
            inner: RefCell::new(WebSocketInner {
                codec: Codec::new().client_mode(),
                eof: false,
                send_buf: BytesMut::new(),
                recv_buf: BytesMut::new(),
                body,
            }),
        })
    }

    /// Split into a sink and reader pair that can be used for concurrent read/write
    /// message to websocket connection.
    #[inline]
    pub fn split<'s>(&'s self) -> (WebSocketSink<'s, 'a>, WebSocketReader<'s, 'a>) {
        (WebSocketSink(self), WebSocketReader(self))
    }

    /// Set max message size.
    ///
    /// By default max size is set to 64kB.
    pub fn max_size(self, size: usize) -> Self {
        let WebSocketInner {
            codec,
            eof,
            send_buf,
            recv_buf,
            body,
        } = self.inner.into_inner();

        Self {
            inner: RefCell::new(WebSocketInner {
                codec: codec.set_max_size(size),
                eof,
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

struct WebSocketInner<'b> {
    codec: Codec,
    eof: bool,
    send_buf: BytesMut,
    recv_buf: BytesMut,
    body: ResponseBody<'b>,
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

        match inner.body {
            ResponseBody::H1(ref mut body) => {
                while !inner.send_buf.chunk().is_empty() {
                    match ready!(Pin::new(&mut **body.conn()).poll_write(cx, inner.send_buf.chunk()))? {
                        0 => return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())),
                        n => inner.send_buf.advance(n),
                    }
                }

                Pin::new(&mut **body.conn()).poll_flush(cx).map_err(Into::into)
            }
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => {
                while !inner.send_buf.chunk().is_empty() {
                    ready!(body.poll_send_buf(&mut inner.send_buf, cx))?;
                }

                Poll::Ready(Ok(()))
            }
            _ => unreachable!(),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.as_mut().poll_flush(cx))?;
        match self.get_mut().body {
            ResponseBody::H1(ref mut body) => Pin::new(&mut **body.conn()).poll_shutdown(cx).map_err(Into::into),
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => {
                body.send_data(xitca_http::bytes::Bytes::new(), true)?;
                Poll::Ready(Ok(()))
            }
            _ => unreachable!(),
        }
    }
}

impl Stream for WebSocketInner<'_> {
    type Item = Result<Message, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Some(msg) = this.codec.decode(&mut this.recv_buf)? {
                return Poll::Ready(Some(Ok(msg)));
            }

            if this.eof {
                return Poll::Ready(None);
            }

            match ready!(Pin::new(&mut this.body).poll_next(cx)) {
                Some(res) => {
                    let bytes = res?;
                    this.recv_buf.extend_from_slice(&bytes);
                }
                None => this.eof = true,
            }
        }
    }
}
