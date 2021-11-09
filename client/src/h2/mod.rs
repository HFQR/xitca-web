mod error;

pub(crate) mod body;
pub(crate) mod proto;

pub use self::error::Error;

use h2::client::SendRequest;
use xitca_http::bytes::Bytes;

pub type Connection = SendRequest<Bytes>;
