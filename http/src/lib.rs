//! Http module for [Service](xitca_service::Service) trait oriented http handling.
//!
//! This crate tries to serve both low overhead and ease of use purpose.
//! All http protocols can be used separately with corresponding feature flag or work together
//! for handling different protocols in one place.

#![forbid(unsafe_code)]
#![feature(async_fn_in_trait, return_position_impl_trait_in_trait)]

#[cfg(feature = "runtime")]
mod builder;
#[cfg(feature = "runtime")]
mod service;
mod tls;
mod version;

pub mod body;
pub mod error;
pub mod http;

#[cfg(feature = "runtime")]
pub mod date;
#[cfg(feature = "http1")]
pub mod h1;
#[cfg(feature = "http2")]
pub mod h2;
#[cfg(feature = "http3")]
pub mod h3;

pub mod config;
pub mod util;

/// re-export bytes crate as module.
pub use xitca_io::bytes;

pub use self::body::{RequestBody, ResponseBody};
pub use self::error::{BodyError, HttpServiceError};
pub use self::http::{Request, Response};
#[cfg(feature = "runtime")]
pub use self::{builder::HttpServiceBuilder, service::HttpService};

// TODO: enable this conflict feature check.
// temporary compile error for conflicted feature combination.
// #[cfg(not(feature = "http1"))]
// #[cfg(all(feature = "http2", feature = "native-tls"))]
// compile_error!("http2 feature can not use native-tls");

pub(crate) fn unspecified_socket_addr() -> std::net::SocketAddr {
    std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::UNSPECIFIED, 0))
}
