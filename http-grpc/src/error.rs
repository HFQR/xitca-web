use core::fmt;

use http::header::{HeaderMap, HeaderValue};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use super::status::GrpcStatus;

const GRPC_STATUS: http::header::HeaderName = http::header::HeaderName::from_static("grpc-status");
const GRPC_MESSAGE: http::header::HeaderName = http::header::HeaderName::from_static("grpc-message");

/// Per gRPC spec: percent-encode `grpc-message` using RFC 3986 unreserved characters.
const GRPC_MESSAGE_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// gRPC protocol errors.
#[derive(Debug)]
pub enum ProtocolError {
    /// The body ended before a complete gRPC frame could be read.
    IncompleteFrame,
    /// A single gRPC message exceeded the configured size limit.
    MessageTooLarge { size: usize, limit: usize },
    /// Protobuf decode failed.
    Decode(prost::DecodeError),
    /// Protobuf encode failed.
    Encode(prost::EncodeError),
    /// Compression flag set but no encoding was configured.
    CompressedWithoutEncoding,
    /// Compression/decompression error.
    Compress(String),
    /// Compression is not supported (feature not enabled).
    CompressUnsupported,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::IncompleteFrame => f.write_str("incomplete grpc frame"),
            Self::MessageTooLarge { size, limit } => {
                write!(f, "grpc message size {size} exceeds limit {limit}")
            }
            Self::Decode(ref e) => write!(f, "protobuf decode: {e}"),
            Self::Encode(ref e) => write!(f, "protobuf encode: {e}"),
            Self::CompressedWithoutEncoding => f.write_str("compressed flag set but no grpc-encoding configured"),
            Self::Compress(ref e) => write!(f, "compression error: {e}"),
            Self::CompressUnsupported => f.write_str("grpc compression not supported"),
        }
    }
}

impl core::error::Error for ProtocolError {}

/// Error type for gRPC operations. Carries a status code and optional message,
/// and can produce gRPC trailers for error responses.
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

    /// Produce gRPC trailers (`grpc-status` and optionally `grpc-message`) from this error.
    pub fn trailers(&self) -> HeaderMap {
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

impl core::error::Error for GrpcError {}
