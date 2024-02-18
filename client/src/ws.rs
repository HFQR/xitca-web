//! websocket request/response handling.

pub use http_ws::Message;

use core::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::sync::Mutex;

use futures_core::stream::Stream;
use futures_sink::Sink;
use http_ws::{Codec, RequestStream, WsError};

use super::{
    body::ResponseBody,
    bytes::{Buf, BytesMut},
    error::Error,
    http::{StatusCode, Version},
    tunnel::TunnelRequest,
};

/// new type of [RequestBuilder] with extended functionality for websocket handling.
pub type WsRequest<'a> = TunnelRequest<'a, marker::WebSocket>;

mod marker {
    pub struct WebSocket;
}

impl<'a> WsRequest<'a> {
    /// Send the request and wait for response asynchronously.
    pub async fn send(self) -> Result<WebSocket<'a>, Error> {
        let res = self.req.send().await?;

        let status = res.status();
        let expect_status = match res.version() {
            Version::HTTP_11 if status != StatusCode::SWITCHING_PROTOCOLS => Some(StatusCode::SWITCHING_PROTOCOLS),
            Version::HTTP_2 if status != StatusCode::OK => Some(StatusCode::OK),
            _ => None,
        };

        if let Some(expect_status) = expect_status {
            return Err(Error::Std(format!("expecting {expect_status}, got {status}").into()));
        }

        let body = res.res.into_body();
        WebSocket::try_from_body(body)
    }
}

/// sender part of websocket connection.
/// [Sink] trait is used to asynchronously send message.
pub struct WebSocketSink<'a, 'b>(&'a WebSocket<'b>);

impl Sink<Message> for WebSocketSink<'_, '_> {
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()).poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()).poll_close(cx)
    }
}

/// sender part of websocket connection.
/// [Stream] trait is used to asynchronously receive message.
pub struct WebSocketReader<'a, 'b>(&'a WebSocket<'b>);

impl Stream for WebSocketReader<'_, '_> {
    type Item = Result<Message, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()).poll_next(cx)
    }
}

/// A unified websocket that can be used as both sender/receiver.
///
/// * This type can not do concurrent message handling which means send always block receive
/// and vice versa.
pub struct WebSocket<'c> {
    inner: Mutex<WebSocketInner<'c>>,
}

impl<'a> WebSocket<'a> {
    pub(crate) fn try_from_body(body: ResponseBody<'a>) -> Result<Self, Error> {
        Ok(Self {
            inner: Mutex::new(WebSocketInner {
                codec: Codec::new().client_mode(),
                send_buf: BytesMut::new(),
                recv_stream: RequestStream::with_codec(body, Codec::new().client_mode()),
            }),
        })
    }

    /// Split into a sink and reader pair that can be used for concurrent read/write
    /// message to websocket connection.
    #[inline]
    pub fn split(&self) -> (WebSocketSink<'_, 'a>, WebSocketReader<'_, 'a>) {
        (WebSocketSink(self), WebSocketReader(self))
    }

    /// Set max message size.
    ///
    /// By default max size is set to 64kB.
    pub fn max_size(mut self, size: usize) -> Self {
        let inner = self.inner.get_mut().unwrap();
        inner.codec = inner.codec.set_max_size(size);
        let recv_codec = inner.recv_stream.codec_mut();
        *recv_codec = recv_codec.set_max_size(size);
        self
    }

    fn get_mut_pinned_inner(self: Pin<&mut Self>) -> Pin<&mut WebSocketInner<'a>> {
        Pin::new(self.get_mut().inner.get_mut().unwrap())
    }
}

impl Sink<Message> for WebSocket<'_> {
    type Error = Error;

    #[inline]
    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut_pinned_inner().poll_ready(cx)
    }

    #[inline]
    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        self.get_mut_pinned_inner().start_send(item)
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut_pinned_inner().poll_flush(cx)
    }

    #[inline]
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut_pinned_inner().poll_close(cx)
    }
}

impl Stream for WebSocket<'_> {
    type Item = Result<Message, Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut_pinned_inner().poll_next(cx)
    }
}

struct WebSocketInner<'b> {
    codec: Codec,
    send_buf: BytesMut,
    recv_stream: RequestStream<ResponseBody<'b>>,
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

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let inner = self.get_mut();

        match inner.recv_stream.inner_mut() {
            #[cfg(feature = "http1")]
            ResponseBody::H1(body) => {
                use std::io;
                use tokio::io::AsyncWrite;
                while !inner.send_buf.chunk().is_empty() {
                    match ready!(Pin::new(&mut **body.conn()).poll_write(_cx, inner.send_buf.chunk()))? {
                        0 => return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())),
                        n => inner.send_buf.advance(n),
                    }
                }

                Pin::new(&mut **body.conn()).poll_flush(_cx).map_err(Into::into)
            }
            #[cfg(feature = "http2")]
            ResponseBody::H2(body) => {
                while !inner.send_buf.chunk().is_empty() {
                    ready!(body.poll_send_buf(&mut inner.send_buf, _cx))?;
                }

                Poll::Ready(Ok(()))
            }
            _ => panic!("websocket can only be enabled when http1 or http2 feature is also enabled"),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.as_mut().poll_flush(cx))?;
        match self.get_mut().recv_stream.inner_mut() {
            #[cfg(feature = "http1")]
            ResponseBody::H1(body) => {
                tokio::io::AsyncWrite::poll_shutdown(Pin::new(&mut **body.conn()), cx).map_err(Into::into)
            }
            #[cfg(feature = "http2")]
            ResponseBody::H2(body) => {
                body.send_data(xitca_http::bytes::Bytes::new(), true)?;
                Poll::Ready(Ok(()))
            }
            _ => panic!("websocket can only be enabled when http1 or http2 feature is also enabled"),
        }
    }
}

impl Stream for WebSocketInner<'_> {
    type Item = Result<Message, Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().recv_stream)
            .poll_next(cx)
            .map_err(|e| match e {
                WsError::Protocol(e) => Error::from(e),
                WsError::Stream(e) => Error::Std(e),
            })
    }
}
