use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll, ready},
};

use futures_core::stream::Stream;
use futures_sink::Sink;
use http_grpc_rs::Codec;
use prost::Message;
use xitca_http::http::{
    HeaderMap, HeaderValue,
    const_header_name::{GRPC_ACCEPT_ENCODING, GRPC_ENCODING},
};

#[cfg(feature = "http2")]
use super::bytes::Buf;
use super::{
    body::{Body, Frame, RequestBody},
    bytes::{Bytes, BytesMut},
    error::{Error, ErrorResponse},
    http::{Request, StatusCode, const_header_value::GRPC, header::CONTENT_TYPE},
    request::RequestBuilder,
    response::Response,
    tunnel::{Tunnel, TunnelSink, TunnelStream},
};

pub use http_encoding::ContentEncoding;

/// Returns `true` if the request carries a gRPC content-type.
///
/// Matches `application/grpc` and any `application/grpc+...` / `application/grpc-...`
/// subtype. Used by the transport layer to short-circuit h2c prior-knowledge fallback
/// to HTTP/1.1: a gRPC request that can't complete an HTTP/2 handshake cannot succeed
/// over HTTP/1.1 either, so surfacing the handshake error directly gives a better
/// diagnostic than letting the response body fail to decode as gRPC framing.
pub(crate) fn is_grpc_request<B>(req: &Request<B>) -> bool {
    req.headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/grpc"))
}

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
            #[cfg(feature = "http2")]
            crate::body::ResponseBody::H2(body) => GrpcBody::H2(body),
            #[cfg(feature = "http3")]
            crate::body::ResponseBody::H3(body) => GrpcBody::H3(body),
            _ => unreachable!("gRPC requires http2 or http3"),
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

#[allow(clippy::large_enum_variant)]
/// Transport-agnostic body enum for gRPC streaming over HTTP/2 or HTTP/3.
enum GrpcBody {
    #[cfg(feature = "http2")]
    H2(crate::h2::body::ResponseBody),
    #[cfg(feature = "http3")]
    H3(crate::h3::body::ResponseBody),
}

impl GrpcBody {
    fn poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Error>>> {
        match self {
            #[cfg(feature = "http2")]
            Self::H2(body) => Pin::new(body).poll_frame(cx).map_err(Into::into),
            #[cfg(feature = "http3")]
            Self::H3(body) => Pin::new(body).poll_frame(cx).map_err(Into::into),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }
}

pub struct GrpcTunnel<M> {
    encode_codec: Codec,
    decode_codec: Codec,
    send_buf: BytesMut,
    recv_buf: BytesMut,
    body: GrpcBody,
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
        match inner.body {
            #[cfg(feature = "http2")]
            GrpcBody::H2(ref mut body) => {
                while !inner.send_buf.chunk().is_empty() {
                    ready!(body.poll_send_buf(&mut inner.send_buf, cx))?;
                }
                Poll::Ready(Ok(()))
            }
            #[cfg(feature = "http3")]
            GrpcBody::H3(ref mut body) => {
                if inner.send_buf.is_empty() {
                    return Poll::Ready(Ok(()));
                }
                let buf = inner.send_buf.split().freeze();
                // h3's send_data is: synchronous buffer into QUIC, then async poll_ready
                // for flow control. If poll_ready returns Pending the data is already
                // queued in QUIC — the flow-control wait is best-effort here because
                // pin! creates a stack-local future that cannot survive across poll calls.
                // TODO: store the future to properly respect QUIC flow control backpressure.
                let mut fut = core::pin::pin!(body.send_data(buf));
                fut.as_mut().poll(cx).map_err(Into::into)
            }
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let inner = self.get_mut();
        match inner.body {
            #[cfg(feature = "http2")]
            GrpcBody::H2(ref mut body) => {
                while !inner.send_buf.chunk().is_empty() {
                    ready!(body.poll_send_buf(&mut inner.send_buf, cx))?;
                }
                body.send_data(Bytes::new(), true)?;
                Poll::Ready(Ok(()))
            }
            #[cfg(feature = "http3")]
            GrpcBody::H3(ref mut body) => {
                if !inner.send_buf.is_empty() {
                    let buf = inner.send_buf.split().freeze();
                    let mut fut = core::pin::pin!(body.send_data(buf));
                    ready!(fut.as_mut().poll(cx)).map_err(Error::from)?;
                }
                let mut fut = core::pin::pin!(body.finish());
                fut.as_mut().poll(cx).map_err(Into::into)
            }
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
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

            // Need more data — poll the body.
            match ready!(inner.body.poll_frame(cx)) {
                Some(Ok(Frame::Data(data))) => inner.recv_buf.extend_from_slice(data.as_ref()),
                Some(Ok(Frame::Trailers(_))) => {
                    // TODO: check grpc-status trailer for server errors.
                    return Poll::Ready(None);
                }
                Some(Err(e)) => return Poll::Ready(Some(Err(e))),
                None => {
                    return if inner.recv_buf.is_empty() {
                        Poll::Ready(None)
                    } else {
                        Poll::Ready(Some(Err(Error::Std(Box::new(
                            http_grpc_rs::ProtocolError::IncompleteFrame,
                        )))))
                    };
                }
            }
        }
    }
}
