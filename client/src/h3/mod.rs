mod error;

pub(crate) mod body;
pub(crate) mod proto;

pub use error::Error;

use h3::client;

pub type Connection = client::SendRequest<h3_quinn::OpenStreams>;
