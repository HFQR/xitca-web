mod error;

pub(crate) mod body;
pub(crate) mod proto;

pub use error::Error;

use h3::client;

pub type Connection = client::Connection<h3_quinn::Connection>;
