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

use http_grpc_rs::stream::RequestStream;
use prost::Message;

pub use http_grpc_rs::{
    codec::{Codec, DEFAULT_LIMIT},
    error::{GrpcError, ProtocolError},
    status::GrpcStatus,
    stream::ResponseBody as GrpcStreamResponse,
};

use crate::{
    body::{BodyStream, RequestBody, ResponseBody},
    context::WebContext,
    error::Error,
    handler::{FromRequest, Responder},
    http::WebResponse,
};

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
        let mut stream = from_ctx::<_, _, LIMIT>(ctx)?;
        let msg = stream
            .message()
            .await
            .map_err(Error::from_service)?
            .ok_or_else(|| Error::from_service(GrpcError::new(GrpcStatus::Internal, "empty grpc request body")))?;
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
        GrpcStreamResponse::once(self.0).respond(ctx).await
    }
}

/// Client-streaming extractor that yields a stream of `T: Message` from the request body.
///
/// Wraps [`http_grpc::stream::RequestStream`] with the framework's `RequestBody` type.
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
/// ```
pub type GrpcStreamRequest = RequestStream<RequestBody>;

fn from_ctx<C, B, const LIMIT: usize>(ctx: &WebContext<'_, C, B>) -> Result<GrpcStreamRequest, Error>
where
    B: BodyStream + Default + 'static,
{
    let body = ctx.take_body_ref();
    let body = crate::body::downcast_body(body);

    let mut stream = RequestStream::new(ctx.req().headers(), body);
    stream.codec_mut().set_limit(LIMIT);
    Ok(stream)
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for GrpcStreamRequest
where
    B: BodyStream + Default + 'static,
{
    type Type<'b> = GrpcStreamRequest;
    type Error = Error;

    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        from_ctx::<_, _, DEFAULT_LIMIT>(ctx)
    }
}

/// Send gRPC messages to a [`GrpcStreamResponse`].
pub use http_grpc_rs::stream::ResponseSender as GrpcStreamSender;

impl<'r, C, B, T> Responder<WebContext<'r, C, B>> for http_grpc_rs::stream::ResponseBody<T>
where
    T: Message + Unpin + 'static,
{
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, mut ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let req = core::mem::take(ctx.req_mut()).map(|_| ());
        let res = self.into_response(req);
        Ok(res.map(ResponseBody::boxed))
    }

    fn map(self, _: Self::Response) -> Result<Self::Response, Self::Error>
    where
        Self: Sized,
    {
        panic!(
            "GrpcStreamResponse must be the first item from a series of Responder type. It needs WebContext for content encoding"
        )
    }
}
