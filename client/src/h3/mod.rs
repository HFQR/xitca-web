mod error;

pub(crate) mod body;
pub(crate) mod proto;

pub use error::Error;

use h3::client;
use h3_quinn::OpenStreams;
use xitca_http::bytes::Bytes;

pub type Connection = client::SendRequest<OpenStreams, Bytes>;
