//! middleware offer extended functionality to http client.

mod redirect;
mod retry_closed_connection;

#[cfg(feature = "compress")]
mod decompress;

#[cfg(feature = "compress")]
pub use decompress::Decompress;

pub use redirect::FollowRedirect;
pub use retry_closed_connection::RetryClosedConnection;
