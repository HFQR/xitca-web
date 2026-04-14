use super::{frame::reason::Reason, hpack::DecoderError};

#[derive(Debug)]
pub enum Error {
    MalformedMessage,
    GoAway(Reason),
    Hpack(DecoderError),
    /// Stream-level error: send RST_STREAM(reason) to the peer. Does not close
    /// the connection.
    Reset(Reason),
}

impl From<DecoderError> for Error {
    fn from(e: DecoderError) -> Self {
        Self::Hpack(e)
    }
}
