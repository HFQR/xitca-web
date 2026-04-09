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
    /// the connection. If the third field is `Some(n)`, also send a connection-level
    /// WINDOW_UPDATE(n) to replenish bytes consumed by a discarded DATA payload.
    Reset(StreamId, Reason, Option<usize>),
}

impl From<DecoderError> for Error {
    fn from(e: DecoderError) -> Self {
        Self::Hpack(e)
    }
}
