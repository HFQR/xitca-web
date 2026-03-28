use std::io;

use super::{hpack::DecoderError, reason::Reason, stream_id::StreamId};

#[derive(Debug)]
pub enum Error {
    MalformedMessage,
    GoAway(Reason),
    Hpack(DecoderError),
    Io(io::Error),
    /// Peer sent GOAWAY with a non-NO_ERROR reason, accusing us of a connection-level
    /// protocol violation. Close immediately without sending a GOAWAY reply.
    PeerAccused,
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

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
