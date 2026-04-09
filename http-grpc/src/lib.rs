//! gRPC protocol using high level API that operates over `http_body_alt::Body` trait.
//!
//! # HTTP type
//! - `http` crate types are used for input and output
//! - designed for use with `http/2`

pub mod codec;
pub mod error;
pub mod status;

pub use self::{
    codec::{Codec, DEFAULT_LIMIT},
    error::{GrpcError, ProtocolError},
    status::GrpcStatus,
};

#[cfg(feature = "stream")]
pub mod stream;

#[cfg(feature = "stream")]
pub use self::stream::{RequestStream, ResponseBody, ResponseSender};
