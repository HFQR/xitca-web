//! Http module for [Service](crate::service::Service) trait oriented http handling.

mod body;
mod builder;
mod error;
mod flow;
mod request;
mod tls;

pub mod h2;

pub use builder::HttpServiceBuilder;
pub use error::HttpServiceError;
pub use request::HttpRequest;
