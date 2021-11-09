mod error;

pub(crate) mod body;
pub(crate) mod proto;

pub use error::Error;

use h3::client;
use h3_quinn::quinn::crypto::rustls::TlsSession;

pub type Connection = client::Connection<h3_quinn::Connection<TlsSession>>;
