//! Http module for [Service](xitca_service::Service) trait oriented http handling.
//!
//! This crate tries to serve both low overhead and ease of use purpose.
//! All http protocols can be used separately with corresponding feature flag or work together
//! for handling different protocols in one place.
//!
//! # Examples
//! ```no_run
//! use std::convert::Infallible;
//!
//! use xitca_http::{
//!     http::{IntoResponse, Request, RequestExt, Response},
//!     HttpServiceBuilder,
//!     RequestBody,
//!     ResponseBody
//! };
//! use xitca_service::{fn_service, Service, ServiceExt};
//!
//! # fn main() -> std::io::Result<()> {
//! // xitca-http has to run inside a tcp/udp server.
//! xitca_server::Builder::new()
//!     // create http service with given name, socket address and service logic.
//!     .bind("xitca-http", "localhost:0",
//!         // a simple async function service produce hello world string as http response.
//!         fn_service(|req: Request<RequestExt<RequestBody>>| async {
//!             Ok::<Response<ResponseBody>, Infallible>(req.into_response("Hello,World!"))
//!         })
//!         // http service builder is a middleware take control of above function service
//!         // and bridge tcp/udp transport with the http service.
//!         .enclosed(HttpServiceBuilder::new())
//!     )?
//!     .build()
//! # ; Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

#[cfg(feature = "runtime")]
mod builder;
#[cfg(feature = "runtime")]
mod service;
#[cfg(feature = "runtime")]
mod version;

pub mod body;
pub mod config;
pub mod error;
pub mod http;
pub mod tls;
pub mod util;

#[cfg(feature = "runtime")]
pub mod date;
#[cfg(feature = "http1")]
pub mod h1;
#[cfg(feature = "http2")]
pub mod h2;
#[cfg(feature = "http3")]
pub mod h3;

/// re-export bytes crate as module.
pub use xitca_io::bytes;

pub use self::{
    body::{RequestBody, ResponseBody},
    error::{BodyError, HttpServiceError},
    http::{Request, Response},
};

#[cfg(feature = "runtime")]
pub use self::builder::HttpServiceBuilder;

// TODO: enable this conflict feature check.
// temporary compile error for conflicted feature combination.
// #[cfg(not(feature = "http1"))]
// #[cfg(all(feature = "http2", feature = "native-tls"))]
// compile_error!("http2 feature can not use native-tls");

pub(crate) fn unspecified_socket_addr() -> core::net::SocketAddr {
    core::net::SocketAddr::V4(core::net::SocketAddrV4::new(std::net::Ipv4Addr::UNSPECIFIED, 0))
}
