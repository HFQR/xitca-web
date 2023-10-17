#![forbid(unsafe_code)]

mod body;
mod builder;
mod client;
mod connect;
mod connection;
mod date;
mod pool;
mod request;
mod resolver;
mod response;
mod timeout;
mod tls;
mod uri;

#[cfg(feature = "http1")]
mod h1;

#[cfg(feature = "http2")]
mod h2;

#[cfg(feature = "http3")]
mod h3;

#[cfg(feature = "websocket")]
pub mod ws;

pub mod error;

pub use self::builder::ClientBuilder;
pub use self::client::Client;
pub use self::request::Request;
pub use self::resolver::Resolve;
pub use self::response::Response;
pub use self::tls::{connector::TlsConnect, stream::Io};

// re-export http crate.
pub use xitca_http::http;

// re-export bytes crate.
pub use xitca_http::bytes;
