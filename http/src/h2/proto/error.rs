use super::{
    frame::{reason::Reason, stream_id::StreamId},
    hpack::DecoderError,
};

#[derive(Debug)]
pub enum Error {
    MalformedMessage,
    GoAway(Reason),
    Hpack(DecoderError),
    /// Stream-level error: send RST_STREAM(reason) to the peer. Does not close
    /// the connection.
    Reset(StreamId, Reason),
}

impl From<DecoderError> for Error {
    fn from(e: DecoderError) -> Self {
        Self::Hpack(e)
    }
}
