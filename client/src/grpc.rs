use core::{
    fmt,
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
    const_header_name::{GRPC_ACCEPT_ENCODING, GRPC_ENCODING, GRPC_MESSAGE, GRPC_STATUS},
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

/// How a server ended a call, when it ended it with a failure.
///
/// gRPC reports failure **in the trailers**, not in the response status: the
/// head is long since sent and says 200. A reader that ignores them cannot tell
/// a stream that finished from one that was rejected, aborted or timed out,
/// which turns every server-side failure into a silent end of stream.
///
/// Reaches a caller as [`Error::Grpc`], from both the unary and the streaming
/// path.
#[derive(Debug)]
pub struct GrpcStatusError {
    /// The numeric code from `grpc-status`. Never 0 -- that is a clean end and
    /// is not reported as an error.
    pub status: u16,
    /// `grpc-message`, percent-decoded. Absent when the server sent none.
    pub message: Option<String>,
}

impl fmt::Display for GrpcStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "grpc stream ended with status {}", self.status)?;
        match self.message {
            Some(ref message) => write!(f, ": {message}"),
            None => Ok(()),
        }
    }
}

impl core::error::Error for GrpcStatusError {}

/// gRPC's `UNKNOWN`, which the spec asks a client to report when a server said
/// how it ended but not in a way that can be read.
const GRPC_STATUS_UNKNOWN: u16 = 2;

/// Read how a server ended the stream out of the headers it ended it with.
///
/// Takes a [`HeaderMap`] rather than trailers specifically, because gRPC ends a
/// stream in **two** shapes: trailers after a body, or -- when the server
/// refuses before sending anything -- `grpc-status` in the response head itself,
/// with no body and no trailers at all ("Trailers-Only"). Both have to be read
/// the same way, or the second reads as a stream that simply finished.
///
/// `None` only for a genuine clean end: no `grpc-status` at all, or one that
/// says 0. A status that is present but unreadable is **not** success -- the
/// server was telling us something -- so it is reported as `UNKNOWN`.
fn status_error(trailers: &HeaderMap) -> Option<GrpcStatusError> {
    let status = trailers.get(&GRPC_STATUS)?;
    let status = match status.to_str().ok().and_then(|s| s.parse::<u16>().ok()) {
        Some(0) => return None,
        Some(status) => status,
        None => GRPC_STATUS_UNKNOWN,
    };
    let message = trailers.get(&GRPC_MESSAGE).and_then(|value| {
        let value = value.to_str().ok()?;
        Some(
            percent_encoding::percent_decode_str(value)
                .decode_utf8_lossy()
                .into_owned(),
        )
    });
    Some(GrpcStatusError { status, message })
}

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
    /// Decode the reply to a unary call.
    ///
    /// **A gRPC failure is not an http status.** The head is sent before the
    /// server knows how the call went, so it says `200` either way and the
    /// verdict arrives in `grpc-status`: in the trailers after a reply, or --
    /// when the server refuses before sending anything -- in the head itself,
    /// with no body and no trailers at all ("Trailers-Only"). Both are checked
    /// here, and the head first: reading only the trailers turns every refusal
    /// into an empty response and loses the one thing the server was saying.
    ///
    /// The returned [`HeaderMap`] is trailing **metadata** on a call that
    /// succeeded. It is not where a failure lives; that is [`Error::Grpc`].
    pub async fn grpc<T: Message + Default>(self) -> Result<(T, Option<HeaderMap>), Error> {
        if self.res.status() != StatusCode::OK {
            let (head, _) = self.res.into_parts();
            return Err(Error::from(ErrorResponse {
                expect_status: StatusCode::OK,
                res: crate::http::Response::from_parts(head, ()),
                description: "grpc call can't be completed",
            }));
        }
        let codec = codec_from_head(self.res.headers())?;

        let (mut body, trailers) = self.collect().await?;

        // A failure the server raised after committing to a head. There is no
        // message behind one, so this is read *before* the decode: the other
        // order reports a real status as an empty response.
        if let Some(e) = trailers.as_ref().and_then(status_error) {
            return Err(e.into());
        }

        let msg = codec
            .decode(&mut body)
            .map_err(|e| Error::Std(Box::new(e)))?
            .ok_or_else(|| Error::Std("empty grpc response".into()))?;

        Ok((msg, trailers))
    }
}

impl GrpcStreamRequest<'_> {
    /// Open the stream. Returns as soon as the request is on the wire.
    ///
    /// **This does not wait for the server's response head**, and it must not:
    /// gRPC allows the client to send first, and a server that authenticates a
    /// handshake cannot answer until it has read. Waiting here for a head that
    /// waits on our first message is a deadlock, so the returned [`Grpc`] is
    /// writable immediately and the head is resolved by the first read.
    ///
    /// The cost is that a stream the server refuses outright surfaces as an
    /// error from the reader rather than from here.
    pub async fn send<M>(mut self) -> Result<Grpc<M>, Error> {
        let encode_codec = self.prepare_codec();

        let res = self.builder._send().await?;
        let (head, body) = res.res.into_parts();

        // **Both transports hand the head to the reader**, which is where it is
        // checked. Only http/2 actually defers it -- h3 has the real one right
        // here and carries it along -- but going through one path means the
        // status, the Trailers-Only case and the body encoding are handled
        // once, for both.
        let body = match body {
            #[cfg(feature = "http2")]
            crate::body::ResponseBody::H2(body) => {
                // The head here is the dispatcher's placeholder. The real one
                // has not been sent yet and is collected by the first read.
                drop(head);
                GrpcBody::H2(body)
            }
            #[cfg(feature = "http3")]
            crate::body::ResponseBody::H3(body) => GrpcBody::H3 { body, head: Some(head) },
            _ => unreachable!("gRPC requires http2 or http3"),
        };

        Ok(Tunnel::new(GrpcTunnel {
            encode_codec,
            decode_codec: DecodeCode::Head,
            send_buf: BytesMut::new(),
            recv_buf: BytesMut::new(),
            body,
            _msg: PhantomData,
        }))
    }
}

/// Everything a response head has to be checked for before a stream can be read
/// from, yielding the codec its body is encoded with.
///
/// Three ways a stream can be over before it starts, and all of them arrive
/// here: a transport-level status that is not 200, a `grpc-status` in the head
/// (the Trailers-Only shape a server uses to refuse outright), and a body whose
/// encoding has to be known before a single frame can be decoded.
fn head_codec(head: &crate::http::response::Parts) -> Result<Codec, Error> {
    if head.status != StatusCode::OK {
        return Err(Error::from(ErrorResponse {
            expect_status: StatusCode::OK,
            res: crate::http::Response::from_parts(head.clone(), ()),
            description: "grpc stream can't be established",
        }));
    }
    codec_from_head(&head.headers)
}

/// The Trailers-Only check, and the encoding the body will use, from a head
/// already known to be `200`.
///
/// Shared by the unary and streaming paths. They differ only in how they report
/// a head that was *not* 200 -- the one part of the check that needs the whole
/// head rather than its headers.
fn codec_from_head(headers: &HeaderMap) -> Result<Codec, Error> {
    match status_error(headers) {
        Some(e) => Err(e.into()),
        None => Ok(Codec::from_headers(headers)),
    }
}

#[allow(clippy::large_enum_variant)]
/// Transport-agnostic body enum for gRPC streaming over HTTP/2 or HTTP/3.
enum GrpcBody {
    #[cfg(feature = "http2")]
    H2(crate::h2::body::ResponseBody),
    #[cfg(feature = "http3")]
    H3 {
        body: crate::h3::body::ResponseBody,
        /// h3 never defers, so this is the real head, carried until the reader
        /// asks for it the same way it asks http/2 for one that had to wait.
        head: Option<crate::http::response::Parts>,
    },
}

impl GrpcBody {
    /// Wait for the response head, and hand it over the once it resolves here.
    ///
    /// `Ok(None)` means the head came with the response and the caller already
    /// has it -- the http/3 path, and any future one that does not hand the
    /// body over early.
    fn poll_head(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<crate::http::response::Parts>, Error>> {
        match self {
            #[cfg(feature = "http2")]
            Self::H2(body) => {
                ready!(body.poll_head(cx)).map_err(Error::from)?;
                Poll::Ready(Ok(body.take_head()))
            }
            #[cfg(feature = "http3")]
            Self::H3 { head, .. } => {
                // Already in hand, so there is nothing to wait for and nothing
                // to wake on.
                let _ = cx;
                Poll::Ready(Ok(head.take()))
            }
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    fn poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Error>>> {
        match self {
            #[cfg(feature = "http2")]
            Self::H2(body) => Pin::new(body).poll_frame(cx).map_err(Into::into),
            #[cfg(feature = "http3")]
            Self::H3 { body, .. } => Pin::new(body).poll_frame(cx).map_err(Into::into),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }
}

pub struct GrpcTunnel<M> {
    encode_codec: Codec,
    decode_codec: DecodeCode,
    send_buf: BytesMut,
    recv_buf: BytesMut,
    body: GrpcBody,
    _msg: PhantomData<fn(M)>,
}

/// The codec used to decode what the server sends, which is built from its
/// response head -- and so does not exist until that head does.
///
/// Not an `Option`, because "no codec yet" and "no codec at all" are not the
/// same thing: the head is still owed, and reading is what collects it.
enum DecodeCode {
    /// The head has not been taken off the body yet.
    Head,
    Body(Codec),
    /// The head said the stream was never established. There is nothing to
    /// decode and never will be, and **the head has already been consumed** --
    /// without this, polling again after the error resolves nothing, falls
    /// through to a default codec and starts reading the error body as gRPC
    /// frames.
    Refused,
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
            GrpcBody::H3 { ref mut body, .. } => {
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
            GrpcBody::H3 { ref mut body, .. } => {
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

        let decode_codec = match inner.decode_codec {
            DecodeCode::Body(ref codec) => codec,
            // Reported once, when it happened. Anything further is the caller
            // polling past an error, and there is nothing left to give it.
            DecodeCode::Refused => return Poll::Ready(None),
            DecodeCode::Head => {
                let head = match ready!(inner.body.poll_head(cx)) {
                    Ok(Some(head)) => head,
                    // Taken once, and this state is left the moment it is.
                    Ok(None) => unreachable!("the head is resolved exactly once"),
                    Err(e) => {
                        inner.decode_codec = DecodeCode::Refused;
                        return Poll::Ready(Some(Err(e)));
                    }
                };
                match head_codec(&head) {
                    Ok(codec) => {
                        inner.decode_codec = DecodeCode::Body(codec);
                        match inner.decode_codec {
                            DecodeCode::Body(ref codec) => codec,
                            _ => unreachable!("just set to Body"),
                        }
                    }
                    Err(e) => {
                        inner.decode_codec = DecodeCode::Refused;
                        return Poll::Ready(Some(Err(e)));
                    }
                }
            }
        };

        loop {
            // Try to decode a complete message from the buffer first.
            match decode_codec.decode::<T>(&mut inner.recv_buf) {
                Ok(Some(msg)) => return Poll::Ready(Some(Ok(msg))),
                Ok(None) => {}
                Err(e) => return Poll::Ready(Some(Err(Error::Std(Box::new(e))))),
            }

            // Need more data — poll the body.
            match ready!(inner.body.poll_frame(cx)) {
                Some(Ok(Frame::Data(data))) => inner.recv_buf.extend_from_slice(data.as_ref()),
                // How the server ended the stream. A failure lives here rather
                // than in the response status, which was sent long before the
                // server knew -- so dropping these turns every server-side
                // error into an ordinary end of stream.
                Some(Ok(Frame::Trailers(trailers))) => {
                    return match status_error(&trailers) {
                        Some(e) => Poll::Ready(Some(Err(e.into()))),
                        None => Poll::Ready(None),
                    };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn trailers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                crate::http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn a_clean_end_is_not_an_error() {
        assert!(status_error(&trailers(&[("grpc-status", "0")])).is_none());
        // Some servers omit the trailer entirely on success.
        assert!(status_error(&trailers(&[])).is_none());
    }

    /// A status nothing can be made of is not success. The server was telling
    /// us something, and reading it as a clean end loses whatever it was.
    #[test]
    fn an_unreadable_status_is_unknown_not_success() {
        let err = status_error(&trailers(&[("grpc-status", "not a number")]))
            .expect("a status that cannot be read is not a clean end");
        assert_eq!(err.status, GRPC_STATUS_UNKNOWN);
    }

    #[test]
    fn a_failure_keeps_its_status_and_message() {
        let err = status_error(&trailers(&[
            ("grpc-status", "14"),
            ("grpc-message", "went away mid-stream"),
        ]))
        .expect("a non-zero status is a failure");
        assert_eq!(err.status, 14);
        assert_eq!(err.message.as_deref(), Some("went away mid-stream"));
    }

    /// The spec percent-encodes `grpc-message`, so a reader that does not decode
    /// it shows the operator escapes instead of the reason.
    #[test]
    fn the_message_is_percent_decoded() {
        let err = status_error(&trailers(&[
            ("grpc-status", "9"),
            ("grpc-message", "r%C3%A9ponse%20sp%C3%A9ciale%20%E2%98%95"),
        ]))
        .expect("a non-zero status is a failure");
        assert_eq!(err.message.as_deref(), Some("réponse spéciale ☕"));
    }

    /// The point of the [`Error::Grpc`] variant: a caller reads the code by
    /// matching, not by downcasting a boxed `dyn Error`.
    #[test]
    fn a_refusal_in_the_head_reaches_the_caller_as_a_matchable_error() {
        let err = codec_from_head(&trailers(&[("grpc-status", "5"), ("grpc-message", "no%20such%20file")]))
            .err()
            .expect("a Trailers-Only head has no body to build a codec for");
        match err {
            Error::Grpc(status) => {
                assert_eq!(status.status, 5);
                assert_eq!(status.message.as_deref(), Some("no such file"));
            }
            other => panic!("expected Error::Grpc, got {other:?}"),
        }
    }

    /// A head that says nothing about the status is an ordinary head, and the
    /// body behind it is read with whatever encoding it names.
    #[test]
    fn an_ordinary_head_yields_a_codec() {
        assert!(codec_from_head(&trailers(&[])).is_ok());
        assert!(codec_from_head(&trailers(&[("grpc-status", "0")])).is_ok());
    }

    /// While the status was boxed into `Error::Std`, `source` reached the
    /// *inner* error's source rather than the status, so a caller walking the
    /// chain never saw it at all.
    #[test]
    fn the_status_is_reachable_through_source() {
        let err = Error::from(GrpcStatusError {
            status: 14,
            message: Some("unavailable".to_owned()),
        });
        let source = core::error::Error::source(&err).expect("the status is the source");
        assert!(source.is::<GrpcStatusError>());
    }

    /// A status with no message is still a status.
    #[test]
    fn a_message_is_optional() {
        let err = status_error(&trailers(&[("grpc-status", "4")])).expect("a non-zero status");
        assert_eq!(err.status, 4);
        assert!(err.message.is_none());
    }
}
