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
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::Stream;
use prost::Message;

#[cfg(feature = "__compress")]
use http_encoding::ContentEncoding;

use crate::{
    body::{Body, BodyExt, BodyStream, Frame, RequestBody, ResponseBody},
    bytes::{BufMut, Bytes, BytesMut},
    context::WebContext,
    error::{BodyError, Error, GrpcError, GrpcStatus},
    handler::{FromRequest, Responder},
    http::{
        WebResponse,
        const_header_name::GRPC_STATUS,
        const_header_value::GRPC,
        header::{CONTENT_TYPE, HeaderMap, HeaderValue},
    },
};

#[cfg(feature = "__compress")]
use crate::http::const_header_name::{GRPC_ACCEPT_ENCODING, GRPC_ENCODING};

/// Default body size limit for gRPC messages (4 MiB).
pub const DEFAULT_LIMIT: usize = 4 * 1024 * 1024;

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
    B: BodyStream + Default + Unpin + 'static,
    T: Message + Default,
{
    type Type<'b> = Grpc<T, LIMIT>;
    type Error = Error;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let mut stream = GrpcStream::<T, RequestBody, LIMIT>::from_request(ctx).await?;
        let msg = stream
            .message()
            .await?
            .ok_or_else(|| GrpcError::new(GrpcStatus::Internal, "empty grpc request body"))?;
        Ok(Grpc(msg))
    }
}

impl<'r, C, B, T, const LIMIT: usize> Responder<WebContext<'r, C, B>> for Grpc<T, LIMIT>
where
    T: Message + Unpin + 'static,
{
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        GrpcStream::<T, Once<T>, 0>::once(self.0).respond(ctx).await
    }
}

/// Streaming type for gRPC methods.
///
/// Wraps a stream `S` that yields `Result<T, E>` items and encodes each `T: Message` with gRPC
/// length-prefixed framing, appending `grpc-status: 0` trailers on completion.
///
/// For the extractor (client-streaming) use case, see [`GrpcStreamRequest`].
///
/// # Example
/// ```ignore
/// use xitca_web::handler::grpc::{Grpc, GrpcStream};
///
/// async fn server_stream(Grpc(req): Grpc<MyRequest>) -> GrpcStream<MyReply, impl Stream<Item = Result<MyReply, Error>>> {
///     GrpcStream::new(async_stream::stream! {
///         yield Ok(MyReply { .. });
///         yield Ok(MyReply { .. });
///     })
/// }
/// ```
pub struct GrpcStream<T, S = RequestBody, const LIMIT: usize = DEFAULT_LIMIT> {
    value: S,
    #[cfg(feature = "__compress")]
    encoding: ContentEncoding,
    buf: BytesMut,
    done: bool,
    _msg: PhantomData<fn() -> T>,
}

/// Client-streaming extractor that yields a stream of `T: Message` from the request body.
///
/// This is a type alias for [`GrpcStream<T, RequestBody, LIMIT>`] that simplifies the
/// common extractor use case. Decodes the gRPC length-prefixed wire format frame by frame,
/// yielding each decoded message via [`GrpcStream::message`].
///
/// `LIMIT` controls the maximum allowed size in bytes for a single gRPC message frame.
/// Defaults to [`DEFAULT_LIMIT`] (4 MiB). Set to `0` for unlimited.
///
/// # Example
/// ```ignore
/// use xitca_web::handler::grpc::{GrpcStreamRequest, Grpc};
///
/// // default 4 MiB limit
/// async fn client_stream(mut stream: GrpcStreamRequest<MyRequest>) -> Grpc<MyReply> {
///     while let Some(msg) = stream.message().await? {
///         // process each MyRequest
///     }
///     Grpc(MyReply { .. })
/// }
///
/// // custom 1 MiB limit
/// async fn limited_stream(mut stream: GrpcStreamRequest<MyRequest, { 1024 * 1024 }>) -> Grpc<MyReply> {
///     while let Some(msg) = stream.message().await? {
///         // process each MyRequest
///     }
///     Grpc(MyReply { .. })
/// }
/// ```
pub type GrpcStreamRequest<T, const LIMIT: usize = DEFAULT_LIMIT> = GrpcStream<T, RequestBody, LIMIT>;

impl<T, S, const LIMIT: usize> GrpcStream<T, S, LIMIT> {
    pub fn new(value: S) -> Self {
        Self {
            value,
            #[cfg(feature = "__compress")]
            encoding: ContentEncoding::default(),
            buf: BytesMut::new(),
            done: false,
            _msg: PhantomData,
        }
    }

    #[cfg(feature = "__compress")]
    pub fn set_encoding(mut self, encoding: ContentEncoding) -> Self {
        self.encoding = encoding;
        self
    }

    /// Finalize the gRPC frame in `self.buf`. Optionally compresses the protobuf payload
    /// and writes the compressed flag + length prefix.
    #[cfg(feature = "__compress")]
    fn encode_frame(&mut self) -> Result<(), BodyError> {
        if !matches!(self.encoding, ContentEncoding::Identity) {
            let payload = self.buf.split_off(5);
            let coder = self.encoding.encode_body(crate::body::Full::new(payload));

            // clear buf and write compressed frame header
            self.buf.clear();
            self.buf.put_u8(1); // compressed flag
            self.buf.put_u32(0); // placeholder length

            // drive the coder synchronously — Full yields once and never returns Pending
            let mut coder = core::pin::pin!(coder);
            let waker = core::task::Waker::noop();
            let mut cx = core::task::Context::from_waker(waker);
            loop {
                match Body::poll_frame(coder.as_mut(), &mut cx) {
                    Poll::Ready(Some(Ok(Frame::Data(data)))) => self.buf.extend_from_slice(data.as_ref()),
                    Poll::Ready(Some(Err(err))) => return Err(err),
                    Poll::Ready(None) => break,
                    Poll::Pending | Poll::Ready(Some(Ok(Frame::Trailers(_)))) => {
                        unreachable!("Full body never returns Pending")
                    }
                }
            }
        }

        // write the actual payload length (after the 1-byte flag)
        let len = (self.buf.len() - 5) as u32;
        self.buf[1..5].copy_from_slice(&len.to_be_bytes());

        Ok(())
    }

    #[cfg(not(feature = "__compress"))]
    fn encode_frame(&mut self) -> Result<(), BodyError> {
        let len = (self.buf.len() - 5) as u32;
        self.buf[1..5].copy_from_slice(&len.to_be_bytes());
        Ok(())
    }
}

impl<T, const LIMIT: usize> GrpcStream<T, Once<T>, LIMIT> {
    /// Create a single-item stream for unary responses.
    pub fn once(msg: T) -> Self {
        GrpcStream::new(Once(Some(msg)))
    }
}

/// Single-item stream that yields one `Ok(T)` then completes.
pub struct Once<T>(Option<T>);

impl<T: Unpin> Stream for Once<T> {
    type Item = Result<T, Infallible>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().0.take().map(Ok))
    }
}

impl<T, S, E, const LIMIT: usize> Body for GrpcStream<T, S, LIMIT>
where
    T: Message,
    S: Stream<Item = Result<T, E>> + Unpin,
    E: Into<BodyError>,
{
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        match this._poll_frame(cx) {
            Poll::Ready(Some(Err(e))) => {
                this.done = true;
                let err = GrpcError::new(GrpcStatus::Internal, e.to_string());
                Poll::Ready(Some(Ok(Frame::Trailers(err.trailers()))))
            }
            poll => poll,
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.done
    }
}

impl<T, S, E, const LIMIT: usize> GrpcStream<T, S, LIMIT>
where
    T: Message,
    S: Stream<Item = Result<T, E>> + Unpin,
    E: Into<BodyError>,
{
    fn _poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        if self.done {
            return Poll::Ready(None);
        }

        match Pin::new(&mut self.value).poll_next(cx) {
            Poll::Ready(Some(Ok(msg))) => {
                let encoded_len = msg.encoded_len();
                self.buf.reserve(5 + encoded_len);
                self.buf.put_u8(0); // no compression flag (overwritten below if compressed)
                self.buf.put_u32(0); // placeholder length

                msg.encode(&mut self.buf)
                    .map_err(|e| BodyError::from(Box::new(e) as Box<dyn core::error::Error + Send + Sync>))?;

                self.encode_frame()?;

                Poll::Ready(Some(Ok(Frame::Data(self.buf.split().freeze()))))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(None) => {
                self.done = true;
                let mut trailers = HeaderMap::with_capacity(1);
                trailers.insert(GRPC_STATUS, HeaderValue::from_static("0"));
                Poll::Ready(Some(Ok(Frame::Trailers(trailers))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<'r, C, B, T, S, E, const LIMIT: usize> Responder<WebContext<'r, C, B>> for GrpcStream<T, S, LIMIT>
where
    T: Message + 'static,
    S: futures_core::Stream<Item = Result<T, E>> + Unpin + 'static,
    E: Into<BodyError> + 'static,
{
    type Response = WebResponse;
    type Error = Error;

    async fn respond(mut self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.buf.clear();

        #[cfg(feature = "__compress")]
        let encoding = {
            let encoding = ContentEncoding::from_headers_with(ctx.req().headers(), &GRPC_ACCEPT_ENCODING);
            self = self.set_encoding(encoding);
            encoding
        };

        let mut res = ctx.into_response(ResponseBody::boxed(self));

        res.headers_mut().insert(CONTENT_TYPE, GRPC);

        #[cfg(feature = "__compress")]
        if !matches!(encoding, ContentEncoding::Identity) {
            res.headers_mut()
                .insert(GRPC_ENCODING, HeaderValue::from_static(encoding.as_str()));
        }

        Ok(res)
    }

    fn map(self, _: Self::Response) -> Result<Self::Response, Self::Error>
    where
        Self: Sized,
    {
        panic!(
            "GrpcStream must be the first item from a series of Responder type. It needs WebContext for content encoding"
        )
    }
}

impl<T, S, const LIMIT: usize> GrpcStream<T, S, LIMIT> {
    /// Read the next message from the stream.
    ///
    /// Returns `Ok(None)` when the client has closed the stream.
    pub async fn message(&mut self) -> Result<Option<T>, Error>
    where
        T: Message + Default,
        S: Body + Unpin,
        S::Data: AsRef<[u8]>,
        S::Error: Into<BodyError>,
    {
        let mut compressed = false;
        let mut len = 0;

        // accumulate until we have the 5-byte gRPC frame header
        while self.buf.len() < 5 + len {
            match self.value.data().await {
                Some(Ok(data)) => self.buf.extend_from_slice(data.as_ref()),
                Some(Err(e)) => {
                    return Err(GrpcError::new(GrpcStatus::Internal, format!("body read: {}", e.into())).into());
                }
                None => {
                    // stream ended; empty buf means clean EOF, partial buf means truncated frame
                    return if self.buf.is_empty() {
                        Ok(None)
                    } else {
                        Err(GrpcError::new(GrpcStatus::Internal, "incomplete grpc frame header").into())
                    };
                }
            }

            if len == 0
                && let Some(prefix) = self.buf.get(1..5)
            {
                compressed = self.buf[0] != 0;
                len = u32::from_be_bytes(prefix.try_into().unwrap()) as usize;

                if LIMIT > 0 && len > LIMIT {
                    return Err(GrpcError::new(
                        GrpcStatus::ResourceExhausted,
                        format!("grpc message size {len} exceeds limit {LIMIT}"),
                    )
                    .into());
                }
            }
        }

        // split off the header and payload, leaving any remainder for the next message
        let _ = self.buf.split_to(5);
        let mut payload = self.buf.split_to(len);

        if compressed {
            payload = self.decode_compressed(payload).await?;
        };

        let msg = Message::decode(payload)
            .map_err(|e| GrpcError::new(GrpcStatus::Internal, format!("protobuf decode: {e}")))?;

        Ok(Some(msg))
    }

    #[cfg(feature = "__compress")]
    async fn decode_compressed(&self, payload: BytesMut) -> Result<BytesMut, Error> {
        if matches!(self.encoding, ContentEncoding::Identity) {
            return Err(GrpcError::new(GrpcStatus::Internal, "compressed flag set but no grpc-encoding header").into());
        }

        let body = self.encoding.decode_body(crate::body::Full::new(payload));
        let mut body = core::pin::pin!(body);
        let mut out = BytesMut::new();
        while let Some(frame) = body.as_mut().data().await {
            let frame = frame.map_err(|e| GrpcError::new(GrpcStatus::Internal, format!("decompress: {e}")))?;
            out.extend_from_slice(frame.as_ref());
        }

        Ok(out)
    }

    #[cfg(not(feature = "__compress"))]
    async fn decode_compressed(&self, _: BytesMut) -> Result<BytesMut, Error> {
        Err(GrpcError::new(GrpcStatus::Unimplemented, "grpc compression not supported").into())
    }
}

impl<'a, 'r, C, B, T, const LIMIT: usize> FromRequest<'a, WebContext<'r, C, B>> for GrpcStream<T, RequestBody, LIMIT>
where
    B: BodyStream + Default + 'static,
    T: Message + Default,
{
    type Type<'b> = GrpcStream<T, RequestBody, LIMIT>;
    type Error = Error;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let value = ctx.take_body_ref();

        let value = crate::body::downcast_body(value);

        #[allow(unused_mut)]
        let mut stream = GrpcStream::new(value);

        #[cfg(feature = "__compress")]
        {
            let encoding = ContentEncoding::from_headers_with(ctx.req().headers(), &GRPC_ENCODING);
            stream = stream.set_encoding(encoding);
        }

        Ok(stream)
    }
}
