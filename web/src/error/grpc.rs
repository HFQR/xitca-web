use core::{convert::Infallible, error, fmt};

use crate::{
    body::{BodyExt, Empty, ResponseBody, Trailers},
    bytes::Bytes,
    context::WebContext,
    http::{
        WebResponse,
        const_header_name::{GRPC_MESSAGE, GRPC_STATUS},
        const_header_value::GRPC,
        header::{CONTENT_TYPE, HeaderMap, HeaderValue},
    },
    service::Service,
};

use super::Error;

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

    pub(crate) fn trailers(&self) -> HeaderMap {
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

impl error::Error for GrpcError {}

impl<'r, C, B> Service<WebContext<'r, C, B>> for GrpcError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let body = Empty::<Bytes>::new().chain(Trailers::new(self.trailers()));
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
