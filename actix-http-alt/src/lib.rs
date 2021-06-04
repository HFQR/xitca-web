//! Http module for [Service](actix_service_alt::Service) trait oriented http handling.

#![forbid(unsafe_code)]
#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]
// TODO: remove these allow flag.
#![allow(dead_code)]

mod body;
mod builder;
mod error;
mod flow;
mod response;
mod stream;
mod tls;

#[cfg(feature = "http1")]
pub mod h1;
#[cfg(feature = "http2")]
pub mod h2;
#[cfg(feature = "http3")]
pub mod h3;

pub mod util;

/// re-export http crate as module.
pub use http;

pub use body::{RequestBody, ResponseBody};
pub use builder::HttpServiceBuilder;
pub use error::{BodyError, HttpServiceError};
