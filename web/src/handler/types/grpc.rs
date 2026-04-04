//! type extractor and response generator for grpc unary messages.
//!
//! # Extracting and responding
//! ```ignore
//! use xitca_web::handler::grpc::Grpc;
//!
//! async fn say_hello(Grpc(req): Grpc<HelloRequest>) -> Grpc<HelloReply> {
//!     Grpc(HelloReply { response: req.request })
//! }
//! ```
//!
//! # Returning errors
//! ```ignore
//! use xitca_web::handler::grpc::{Grpc, GrpcError, GrpcStatus};
//!
//! async fn say_hello(Grpc(req): Grpc<HelloRequest>) -> Result<Grpc<HelloReply>, GrpcError> {
//!     let name = req.request.ok_or_else(|| GrpcError::new(GrpcStatus::InvalidArgument, "missing request"))?;
//!     Ok(Grpc(HelloReply { response: Some(name) }))
//! }
//! ```

use core::{
    convert::Infallible,
    fmt,
    pin::Pin,
    task::{Context, Poll, ready},
};

use pin_project_lite::pin_project;
use prost::Message;

use crate::{
    body::{Body, BodyStream, Empty, Frame, Full, ResponseBody, SizeHint},
    bytes::{BufMut, Bytes, BytesMut},
    context::WebContext,
    error::Error,
    handler::{FromRequest, Responder},
    http::{
        WebResponse,
        const_header_name::{GRPC_MESSAGE, GRPC_STATUS},
        const_header_value::GRPC,
        header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    },
    service::Service,
};

use super::body::Limit;

/// Default body size limit for gRPC messages (4 MiB).
pub const DEFAULT_LIMIT: usize = 4 * 1024 * 1024;

/// gRPC status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GrpcStatus {
    Ok = 0,
    Cancelled = 1,
    Unknown = 2,
    InvalidArgument = 3,
    DeadlineExceeded = 4,
    NotFound = 5,
    AlreadyExists = 6,
    PermissionDenied = 7,
    ResourceExhausted = 8,
    FailedPrecondition = 9,
    Aborted = 10,
    OutOfRange = 11,
    Unimplemented = 12,
    Internal = 13,
    Unavailable = 14,
    DataLoss = 15,
    Unauthenticated = 16,
}

/// Error type for gRPC handlers. Produces a trailers-only response with the given status code
/// and optional message.
pub struct GrpcError {
    pub status: GrpcStatus,
    pub message: Option<String>,
}

impl GrpcError {
    pub fn new(status: GrpcStatus, msg: impl Into<String>) -> Self {
        Self {
            status,
            message: Some(msg.into()),
        }
    }

    pub fn status(status: GrpcStatus) -> Self {
        Self { status, message: None }
    }

    fn trailers(&self) -> HeaderMap {
        let mut map = HeaderMap::with_capacity(2);
        map.insert(GRPC_STATUS, HeaderValue::from(self.status as u16));
        if let Some(ref msg) = self.message {
            if let Ok(v) = HeaderValue::from_str(msg) {
                map.insert(GRPC_MESSAGE, v);
            }
        }
        map
    }
}

impl fmt::Debug for GrpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrpcError")
            .field("status", &self.status)
            .field("message", &self.message)
            .finish()
    }
}

impl fmt::Display for GrpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gRPC error {:?}", self.status)?;
        if let Some(ref msg) = self.message {
            write!(f, ": {msg}")?;
        }
        Ok(())
    }
}

impl std::error::Error for GrpcError {}

impl<'r, C, B> Service<WebContext<'r, C, B>> for GrpcError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let body = GrpcBody {
            body: Empty::<Bytes>::new(),
            trailers: Some(self.trailers()),
        };
        let mut res = ctx.into_response(ResponseBody::boxed(body));
        res.headers_mut().insert(CONTENT_TYPE, GRPC);
        Ok(res)
    }
}

impl From<GrpcError> for Error {
    fn from(e: GrpcError) -> Self {
        Self::from_service(e)
    }
}

/// Extract and response type for unary gRPC messages.
///
/// As an extractor, decodes the gRPC length-prefixed wire format (1 byte compression flag +
/// 4 byte big-endian length + protobuf payload) from the request body.
///
/// As a responder, encodes the protobuf message with gRPC framing and appends `grpc-status: 0`
/// trailers.
///
/// Const generic `LIMIT` controls the maximum request body size in bytes. Default is [`DEFAULT_LIMIT`].
pub struct Grpc<T, const LIMIT: usize = DEFAULT_LIMIT>(pub T);

impl<'a, 'r, C, B, T, const LIMIT: usize> FromRequest<'a, WebContext<'r, C, B>> for Grpc<T, LIMIT>
where
    B: BodyStream + Default,
    T: Message + Default,
{
    type Type<'b> = Grpc<T, LIMIT>;
    type Error = Error;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let (buf, _) = <(BytesMut, Limit<LIMIT>)>::from_request(ctx).await?;

        if buf.len() < 5 {
            return Err(GrpcError::new(GrpcStatus::Internal, "incomplete grpc frame header").into());
        }

        let compression = buf[0];
        if compression != 0 {
            return Err(GrpcError::new(GrpcStatus::Unimplemented, "grpc compression not supported").into());
        }

        let len = buf[1..5].try_into().unwrap();
        let len = u32::from_be_bytes(len) as usize;

        let buf = buf
            .get(5..5 + len)
            .ok_or_else(|| GrpcError::new(GrpcStatus::Internal, "incomplete grpc message payload"))?;

        let msg =
            Message::decode(buf).map_err(|e| GrpcError::new(GrpcStatus::Internal, format!("protobuf decode: {e}")))?;

        Ok(Grpc(msg))
    }
}

impl<'r, C, B, T, const LIMIT: usize> Responder<WebContext<'r, C, B>> for Grpc<T, LIMIT>
where
    T: Message,
{
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        const COMPRESS: bool = {
            #[cfg(not(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de")))]
            {
                false
            }

            #[cfg(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de"))]
            {
                true
            }
        };

        GrpcMaybeCompress::<T, COMPRESS>(self.0).respond(ctx).await
    }
}

/// Response type for unary gRPC messages with explicit control over compression.
///
/// When `COMPRESS` is `true` and a `compress-*` feature is enabled, the response payload will be
/// compressed according to the client's `grpc-accept-encoding` header. When `false`, compression
/// is always skipped regardless of features or client headers.
///
/// [`Grpc<T>`] automatically selects compression based on enabled features. Use this type directly
/// when you need to opt out on a per-handler basis.
///
/// # Example
/// ```ignore
/// use xitca_web::handler::grpc::{GrpcMaybeCompress, GrpcError};
///
/// // always skip compression for this handler, even if compress-* features are enabled
/// async fn handler(Grpc(req): Grpc<MyRequest>) -> Result<GrpcMaybeCompress<MyReply, false>, GrpcError> {
///     Ok(GrpcMaybeCompress(MyReply { .. }))
/// }
/// ```
pub struct GrpcMaybeCompress<T, const COMPRESS: bool>(pub T);

impl<'r, C, B, T, const COMPRESS: bool> Responder<WebContext<'r, C, B>> for GrpcMaybeCompress<T, COMPRESS>
where
    T: Message,
{
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        // encode protobuf message first (without the 5-byte gRPC prefix)
        let encoded_len = self.0.encoded_len();
        let mut proto_buf = BytesMut::with_capacity(encoded_len);
        self.0
            .encode(&mut proto_buf)
            .map_err(|e| GrpcError::new(GrpcStatus::Internal, format!("protobuf encode: {e}")))?;
        let proto_bytes = proto_buf.freeze();

        let (compressed, encoding_name) = if COMPRESS {
            grpc_compress(ctx.req().headers(), proto_bytes.clone())
        } else {
            (None, None)
        };

        let payload = compressed.as_ref().unwrap_or(&proto_bytes);

        let mut buf = BytesMut::with_capacity(5 + payload.len());
        buf.put_u8(if compressed.is_some() { 1 } else { 0 });
        buf.put_u32(payload.len() as u32);
        buf.extend_from_slice(payload);

        let mut trailers = HeaderMap::with_capacity(1);
        trailers.insert(GRPC_STATUS, HeaderValue::from_static("0"));

        let body = GrpcBody {
            body: Full::new(buf.freeze()),
            trailers: Some(trailers),
        };

        let mut res = ctx.into_response(ResponseBody::boxed(body));
        res.headers_mut().insert(CONTENT_TYPE, GRPC);
        if let Some(name) = encoding_name {
            res.headers_mut().insert(GRPC_ENCODING, HeaderValue::from_static(name));
        }
        Ok(res)
    }
}

const GRPC_ENCODING: HeaderName = HeaderName::from_static("grpc-encoding");

/// Try to compress payload based on `grpc-accept-encoding` request header.
/// Returns the compressed bytes and the encoding name, or (None, None) if no compression applied.
#[cfg(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de"))]
fn grpc_compress(headers: &HeaderMap, payload: Bytes) -> (Option<Bytes>, Option<&'static str>) {
    use core::pin::pin;

    use http_encoding::{ContentEncoding, encoder};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::body::{BodyExt, Full};
    use crate::http::Response;

    const GRPC_ACCEPT_ENCODING: HeaderName = HeaderName::from_static("grpc-accept-encoding");

    // parse grpc-accept-encoding into a synthetic accept-encoding header
    // so ContentEncoding::from_headers can pick the best match
    let encoding = {
        let mut synthetic = HeaderMap::new();
        for val in headers.get_all(GRPC_ACCEPT_ENCODING) {
            synthetic.append(crate::http::header::ACCEPT_ENCODING, val.clone());
        }
        ContentEncoding::from_headers(&synthetic)
    };

    if matches!(encoding, ContentEncoding::NoOp) {
        return (None, None);
    }

    let original_len = payload.len();

    // wrap payload in a Response<Full<Bytes>> and run it through the encoder
    let res = Response::new(Full::new(payload));
    let res = encoder(res, encoding);

    // resolve the encoding name from the content-encoding header the encoder set
    let encoding_name = res
        .headers()
        .get(crate::http::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok());

    let name = match encoding_name {
        Some(n) if n.eq_ignore_ascii_case("gzip") => "gzip",
        Some(n) if n.eq_ignore_ascii_case("deflate") => "deflate",
        Some(n) if n.eq_ignore_ascii_case("br") => "br",
        _ => return (None, None),
    };

    // collect the compressed body synchronously (Full yields once, encoder buffers in memory)
    let mut body = pin!(res.into_body());
    let mut out = BytesMut::new();
    while let Some(Ok(frame)) = body.as_mut().data().now_or_panic() {
        out.extend_from_slice(frame.as_ref());
    }

    // only use compressed output if it's actually smaller
    if out.len() < original_len {
        (Some(out.freeze()), Some(name))
    } else {
        (None, None)
    }
}

#[cfg(not(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de")))]
fn grpc_compress(_: &HeaderMap, _: Bytes) -> (Option<Bytes>, Option<&'static str>) {
    (None, None)
}

pin_project! {
    /// Body type that yields a single gRPC data frame followed by trailers.
    struct GrpcBody<B> {
        #[pin]
        body: B,
        trailers: Option<HeaderMap>,
    }
}

impl<B> Body for GrpcBody<B>
where
    B: Body,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        match ready!(this.body.poll_frame(cx)) {
            Some(Ok(Frame::Trailers(_))) => unreachable!("GrpcBody::<B> MUST not yield trailers from B"),
            None => Poll::Ready(this.trailers.take().map(|trailers| Ok(Frame::Trailers(trailers)))),
            res => Poll::Ready(res),
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream() && self.trailers.is_none()
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}
