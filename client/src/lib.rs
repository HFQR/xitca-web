//! a composable http client
//!
//! # Quick Start
//! ```no_run
//! use xitca_client::{error::Error, Client};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Error> {
//!     // build client with tls enabled.
//!     #[cfg(feature = "rustls")]
//!     let client = Client::builder().rustls().finish();
//!     #[cfg(not(feature = "rustls"))]
//!     let client = Client::builder().finish();
//!     // send get request to google and wait for response.
//!     let res = client.get("https://www.google.com/").send().await?;
//!     // parse streaming response body to bytes.
//!     let body = res.body().await?;
//!     // print the body as lossy string.
//!     Ok(println!("{}", String::from_utf8_lossy(&body)))
//! }
//! ```
//!
//! # Composable
//! - extendable middlewares for pre/post processing
//! - customizable core feature like DNS resolver and TLS transport layer
//!
//! ## Middleware
//! Please reference [ClientBuilder::middleware]
//!
//! ## Customize core feature
//! Please reference [ClientBuilder::resolver] and [ClientBuilder::tls_connector]

#![forbid(unsafe_code)]

mod body;
mod builder;
mod client;
mod connect;
mod connection;
mod date;
pub(crate) mod pool;
mod request;
mod resolver;
mod response;
mod service;
mod timeout;
mod tls;
mod tunnel;
mod upgrade;
mod uri;

#[cfg(feature = "http1")]
mod h1;

#[cfg(feature = "http2")]
mod h2;

#[cfg(feature = "http3")]
mod h3;

#[cfg(all(feature = "grpc", feature = "http2"))]
pub mod grpc;

#[cfg(feature = "websocket")]
pub mod ws;

#[cfg(feature = "multipart")]
pub mod multipart;

pub mod error;
pub mod http_tunnel;
pub mod middleware;

pub use self::builder::ClientBuilder;
pub use self::client::Client;
pub use self::connect::Connect;
pub use self::connection::{ConnectionExclusive, ConnectionKey, ConnectionShared};
pub use self::request::RequestBuilder;
pub use self::response::Response;
pub use self::service::{HttpService, Service, ServiceRequest};
pub use self::timeout::TimeoutConfig;
pub use self::tls::{TlsStream, connector::Connector};

pub use self::pool::service::{ExclusiveLease, Lease, Leaser, PoolRequest, PoolService, SharedLease, SpawnOutCome};

// re-export http crate.
pub use xitca_http::http;

// re-export bytes crate.
pub use xitca_http::bytes;
