//! Http module for [Service](actix_service_alt::Service) trait oriented http handling.

#![forbid(unsafe_code)]
#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

mod body;
mod builder;
mod error;
mod flow;
mod request;
mod response;
mod tls;

pub mod h1;
#[cfg(feature = "http2")]
pub mod h2;
#[cfg(feature = "http3")]
pub mod h3;
pub mod util;

pub use body::ResponseBody;
pub use builder::HttpServiceBuilder;
pub use error::HttpServiceError;
pub use request::HttpRequest;
pub use response::HttpResponse;
