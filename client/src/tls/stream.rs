use tokio::io::{AsyncRead, AsyncWrite};

pub type TlsStream = Box<dyn Io>;

/// A trait impl for all types that impl [AsyncRead], [AsyncWrite], [Send] and [Unpin].
/// Enabling `Box<dyn Io>` trait object usage.
pub trait Io: AsyncRead + AsyncWrite + Send + Unpin {}

impl<S> Io for S where S: AsyncRead + AsyncWrite + Send + Unpin {}
