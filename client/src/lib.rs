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

#[cfg(not(any(feature = "http1", feature = "http2")))]
#[cfg(feature = "websocket")]
compile_error!("websocket can only be enabled when http1 or http2 feature is also enabled");

pub mod error;

pub use self::builder::ClientBuilder;
pub use self::client::Client;
pub use self::resolver::Resolve;
pub use self::tls::{connector::TlsConnect, stream::Io};

// re-export http crate.
pub use xitca_http::http;

// re-export bytes crate.
pub use xitca_http::bytes;
