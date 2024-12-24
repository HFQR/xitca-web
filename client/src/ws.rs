//! websocket request/response handling.

pub use http_ws::Message;

use core::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use futures_sink::Sink;
use http_ws::{Codec, RequestStream, WsError};

use super::{
    body::ResponseBody,
    bytes::{Buf, BytesMut},
    connection::ConnectionExclusive,
    error::{Error, ErrorResponse},
    http::{StatusCode, Version},
    request::RequestBuilder,
    tunnel::{Tunnel, TunnelSink, TunnelStream},
};

/// new type of [RequestBuilder] with extended functionality for websocket handling.
///
/// [RequestBuilder]: crate::RequestBuilder
pub type WsRequest<'a> = RequestBuilder<'a, marker::WebSocket>;

mod marker {
    pub struct WebSocket;
}

/// A unified websocket that can be used as both sender/receiver.
///
/// * This type can not handle concurrent message which means send always block receive and vice versa.
pub type WebSocket = Tunnel<WebSocketTunnel>;

/// sender part of websocket connection.
/// [Sink] trait is used to asynchronously send message.
pub type WebSocketSink<'a> = TunnelSink<'a, WebSocketTunnel>;

/// sender part of websocket connection.
/// [Stream] trait is used to asynchronously receive message.
pub type WebSocketReader<'a> = TunnelStream<'a, WebSocketTunnel>;

impl<'a> WsRequest<'a> {
    /// Send the request and wait for response asynchronously.
    pub async fn send(mut self) -> Result<WebSocket, Error> {
        http_ws::client_request_extend(&mut self.req);

        let res = self._send().await?;

        let status = res.status();
        let expect_status = match res.version() {
            Version::HTTP_11 if status != StatusCode::SWITCHING_PROTOCOLS => Some(StatusCode::SWITCHING_PROTOCOLS),
            Version::HTTP_2 if status != StatusCode::OK => Some(StatusCode::OK),
            _ => None,
        };

        if let Some(expect_status) = expect_status {
            return Err(Error::from(ErrorResponse {
                expect_status,
                status,
                description: "websocket connection can't be established",
            }));
        }

        let body = res.res.into_body();
        Ok(WebSocket::new(WebSocketTunnel {
            codec: Codec::new().client_mode(),
            send_buf: BytesMut::new(),
            recv_stream: RequestStream::with_codec(body, Codec::new().client_mode()),
        }))
    }
}

impl WebSocket {
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
}

pub struct WebSocketTunnel {
    codec: Codec,
    send_buf: BytesMut,
    recv_stream: RequestStream<ResponseBody>,
}

impl Sink<Message> for WebSocketTunnel {
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // TODO: set up a meaningful backpressure limit for send buf.
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let inner = self.get_mut();
        inner.codec.encode(item, &mut inner.send_buf).map_err(Into::into)
    }

    #[allow(unreachable_code)]
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let inner = self.get_mut();

        let _io: &mut ConnectionExclusive = match inner.recv_stream.inner_mut() {
            #[cfg(feature = "http1")]
            ResponseBody::H1(body) => body.conn_mut(),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(body) => body.conn_mut(),
            #[cfg(feature = "http2")]
            ResponseBody::H2(body) => {
                while !inner.send_buf.chunk().is_empty() {
                    ready!(body.poll_send_buf(&mut inner.send_buf, _cx))?;
                }
                return Poll::Ready(Ok(()));
            }
            _ => unimplemented!("websocket can only be enabled when http1 or http2 feature is also enabled"),
        };

        use std::io::{self, Write};
        use xitca_io::io::{AsyncIo, Interest};

        let mut io = Pin::new(_io);

        while !inner.send_buf.chunk().is_empty() {
            match io.as_mut().get_mut().write(inner.send_buf.chunk()) {
                Ok(0) => return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())),
                Ok(n) => {
                    inner.send_buf.advance(n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(io.as_mut().poll_ready(Interest::WRITABLE, _cx))?;
                }
                Err(e) => return Poll::Ready(Err(e.into())),
            }
        }

        while let Err(e) = io.as_mut().get_mut().flush() {
            if e.kind() != io::ErrorKind::WouldBlock {
                return Poll::Ready(Err(e.into()));
            }
            ready!(io.as_mut().poll_ready(Interest::WRITABLE, _cx))?;
        }

        Poll::Ready(Ok(()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.as_mut().poll_flush(cx))?;
        match self.get_mut().recv_stream.inner_mut() {
            #[cfg(feature = "http1")]
            ResponseBody::H1(body) => {
                xitca_io::io::AsyncIo::poll_shutdown(Pin::new(&mut **body.conn_mut()), cx).map_err(Into::into)
            }
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(body) => {
                xitca_io::io::AsyncIo::poll_shutdown(Pin::new(&mut **body.conn_mut()), cx).map_err(Into::into)
            }
            #[cfg(feature = "http2")]
            ResponseBody::H2(body) => {
                body.send_data(xitca_http::bytes::Bytes::new(), true)?;
                Poll::Ready(Ok(()))
            }
            _ => unimplemented!("websocket can only be enabled when http1 or http2 feature is also enabled"),
        }
    }
}

impl Stream for WebSocketTunnel {
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
