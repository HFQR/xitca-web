use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll, ready},
};

use futures_core::stream::Stream;
use futures_sink::Sink;
use http_grpc::Codec;
use prost::Message;
use xitca_http::http::{
    HeaderMap, HeaderValue,
    const_header_name::{GRPC_ACCEPT_ENCODING, GRPC_ENCODING},
};

use super::{
    body::{Body, Frame, RequestBody},
    bytes::{Buf, Bytes, BytesMut},
    error::{Error, ErrorResponse},
    h2::body::ResponseBody,
    http::{StatusCode, const_header_value::GRPC, header::CONTENT_TYPE},
    request::RequestBuilder,
    response::Response,
    tunnel::{Tunnel, TunnelSink, TunnelStream},
};

pub use http_encoding::ContentEncoding;

/// new type of [`RequestBuilder`] with extended functionality for grpc stream handling.
pub type GrpcStreamRequest<'a> = GrpcRequest<'a, marker::GrpcStream>;

pub type GrpcUnaryRequest<'a> = GrpcRequest<'a, marker::GrpcUnary>;

mod marker {
    pub struct GrpcStream;
    pub struct GrpcUnary;
}

/// A unified gRPC stream that can be used as both sender/receiver.
pub type Grpc<O> = Tunnel<GrpcTunnel<O>>;

/// sender part of gRPC stream
/// [Sink] trait is used to asynchronously send message.
pub type GrpcSink<'a, O> = TunnelSink<'a, GrpcTunnel<O>>;

/// receiver part of gRPC stream
/// [Stream] trait is used to asynchronously receive message.
pub type GrpcReader<'a, O> = TunnelStream<'a, GrpcTunnel<O>>;

pub struct GrpcRequest<'a, M> {
    builder: RequestBuilder<'a>,
    content_encoding: ContentEncoding,
    _marker: PhantomData<fn(M)>,
}

impl<'a, M> Deref for GrpcRequest<'a, M> {
    type Target = RequestBuilder<'a>;
    fn deref(&self) -> &Self::Target {
        &self.builder
    }
}

impl<'a, M> DerefMut for GrpcRequest<'a, M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.builder
    }
}

impl<'a, M> GrpcRequest<'a, M> {
    pub(crate) fn new(builder: RequestBuilder<'a>) -> Self {
        Self {
            builder,
            content_encoding: ContentEncoding::default(),
            _marker: PhantomData,
        }
    }

    pub fn set_encoding(&mut self, encoding: ContentEncoding) {
        self.content_encoding = encoding;
    }

    fn prepare_codec(&mut self) -> Codec {
        let encoding = self.content_encoding;
        let headers = self.req.headers_mut();

        headers.insert(CONTENT_TYPE, GRPC);

        if !matches!(encoding, ContentEncoding::Identity) {
            headers.insert(GRPC_ENCODING, encoding.as_header_value());
        }

        headers.insert(
            GRPC_ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br, zstd"),
        );
        Codec::new().set_encoding(encoding)
    }
}

impl GrpcUnaryRequest<'_> {
    /// Send the request and wait for response asynchronously.
    pub async fn send(mut self, msg: impl Message) -> Result<Response, Error> {
        let mut buf = BytesMut::new();
        self.prepare_codec()
            .encode(&msg, &mut buf)
            .map_err(|e| Error::Std(Box::new(e)))?;
        *self.req.body_mut() = RequestBody::bytes(buf.freeze());

        self.builder._send().await
    }
}

impl Response {
    pub async fn grpc<T: Message + Default>(self) -> Result<(T, Option<HeaderMap>), Error> {
        let codec = Codec::from_headers(self.res.headers());

        let (mut body, trailers) = self.collect().await?;
        let msg = codec
            .decode(&mut body)
            .map_err(|e| Error::Std(Box::new(e)))?
            .ok_or_else(|| Error::Std("empty grpc response".into()))?;

        Ok((msg, trailers))
    }
}

impl GrpcStreamRequest<'_> {
    /// Send the request and wait for response asynchronously.
    pub async fn send<M>(mut self) -> Result<Grpc<M>, Error> {
        let encode_codec = self.prepare_codec();

        let res = self.builder._send().await?;

        if res.status() != StatusCode::OK {
            return Err(Error::from(ErrorResponse {
                expect_status: StatusCode::OK,
                res: res.res.map(|_| ()),
                description: "grpc stream can't be established",
            }));
        }

        let decode_codec = Codec::from_headers(res.headers());

        let body = match res.res.into_body() {
            crate::body::ResponseBody::H2(body) => body,
            _ => unreachable!("only HTTP/2 gRPC is supported"),
        };

        Ok(Tunnel::new(GrpcTunnel {
            encode_codec,
            decode_codec,
            send_buf: BytesMut::new(),
            recv_buf: BytesMut::new(),
            body,
            _msg: PhantomData,
        }))
    }
}

pub struct GrpcTunnel<M> {
    encode_codec: Codec,
    decode_codec: Codec,
    send_buf: BytesMut,
    recv_buf: BytesMut,
    body: ResponseBody,
    _msg: PhantomData<fn(M)>,
}

impl<T, M> Sink<T> for GrpcTunnel<M>
where
    T: Message,
{
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // TODO: set up a meaningful backpressure limit for send buf.
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let inner = self.get_mut();
        inner
            .encode_codec
            .encode(&item, &mut inner.send_buf)
            .map_err(|e| Error::Std(Box::new(e)))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let inner = self.get_mut();
        while !inner.send_buf.chunk().is_empty() {
            ready!(inner.body.poll_send_buf(&mut inner.send_buf, cx))?;
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let inner = self.get_mut();
        while !inner.send_buf.chunk().is_empty() {
            ready!(inner.body.poll_send_buf(&mut inner.send_buf, cx))?;
        }
        inner.body.send_data(Bytes::new(), true)?;
        Poll::Ready(Ok(()))
    }
}

impl<T> Stream for GrpcTunnel<T>
where
    T: Message + Default,
{
    type Item = Result<T, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let inner = self.get_mut();

        loop {
            // Try to decode a complete message from the buffer first.
            match inner.decode_codec.decode::<T>(&mut inner.recv_buf) {
                Ok(Some(msg)) => return Poll::Ready(Some(Ok(msg))),
                Ok(None) => {}
                Err(e) => return Poll::Ready(Some(Err(Error::Std(Box::new(e))))),
            }

            // Need more data — poll the H2 body.
            match ready!(Pin::new(&mut inner.body).poll_frame(cx)) {
                Some(Ok(Frame::Data(data))) => inner.recv_buf.extend_from_slice(data.as_ref()),
                Some(Ok(Frame::Trailers(_))) => {
                    // TODO: check grpc-status trailer for server errors.
                    return Poll::Ready(None);
                }
                Some(Err(e)) => return Poll::Ready(Some(Err(Error::Std(e)))),
                None => {
                    return if inner.recv_buf.is_empty() {
                        Poll::Ready(None)
                    } else {
                        Poll::Ready(Some(Err(Error::Std(Box::new(
                            http_grpc::ProtocolError::IncompleteFrame,
                        )))))
                    };
                }
            }
        }
    }
}
