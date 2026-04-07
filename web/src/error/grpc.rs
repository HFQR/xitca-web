use core::{convert::Infallible, error, fmt};

use crate::{
    body::ResponseBody,
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
        use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

        // Per gRPC spec: percent-encode `grpc-message` using RFC 3986 unreserved characters.
        // Keep alphanumeric, '-', '_', '.', '~' and encode everything else.
        const GRPC_MESSAGE_ENCODE_SET: &AsciiSet =
            &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

        let mut map = HeaderMap::with_capacity(2);
        map.insert(GRPC_STATUS, HeaderValue::from(self.status as u16));
        if let Some(ref msg) = self.message {
            let encoded = utf8_percent_encode(msg, GRPC_MESSAGE_ENCODE_SET).to_string();
            if let Ok(v) = HeaderValue::from_str(&encoded) {
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
        // gRPC "Trailers-Only" response: status goes in the response headers
        // (the single HEADERS frame with END_STREAM set).
        let mut res = ctx.into_response(ResponseBody::empty());
        res.headers_mut().insert(CONTENT_TYPE, GRPC);
        res.headers_mut().extend(self.trailers());
        Ok(res)
    }
}

impl From<GrpcError> for Error {
    fn from(e: GrpcError) -> Self {
        Self::from_service(e)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn grpc_message_percent_encoded() {
        let err = GrpcError::new(GrpcStatus::Internal, "error with spaces");
        let trailers = err.trailers();
        let msg = trailers.get(GRPC_MESSAGE).unwrap().to_str().unwrap();
        assert_eq!(msg, "error%20with%20spaces");
    }

    #[test]
    fn grpc_message_non_ascii() {
        let err = GrpcError::new(GrpcStatus::Internal, "café ☕");
        let trailers = err.trailers();
        // non-ASCII bytes should be percent-encoded
        let msg = trailers.get(GRPC_MESSAGE).unwrap().to_str().unwrap();
        assert!(!msg.contains('é'));
        assert!(!msg.contains('☕'));
        assert!(msg.starts_with("caf%C3%A9"));
    }

    #[test]
    fn grpc_message_unreserved_chars_preserved() {
        let err = GrpcError::new(GrpcStatus::Internal, "a-b_c.d~e");
        let trailers = err.trailers();
        let msg = trailers.get(GRPC_MESSAGE).unwrap().to_str().unwrap();
        assert_eq!(msg, "a-b_c.d~e");
    }
}
