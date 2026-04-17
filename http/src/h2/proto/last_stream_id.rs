use core::mem;

use super::{
    error::Error,
    frame::{reason::Reason, stream_id::StreamId},
};

/// Tracks the highest stream id we've accepted from the peer plus whether
/// the boundary is still open to advancement.
///
/// - `Incrementable(id)` is the normal state. New HEADERS may advance the
///   boundary via `try_advance`.
/// - `GoAway(id)` is the post-GOAWAY state (RFC 7540 §6.8). The value
///   is frozen and serves as the boundary for silently dropping HEADERS
///   for higher stream ids; `try_set` becomes a no-op.
///
/// All read paths (DATA / WINDOW_UPDATE / RST_STREAM / trailer routing)
/// use `get` and treat both variants identically — they only need the
/// value to detect frames addressed to idle stream ids (RFC 7540 §5.1).
#[derive(Clone, Copy)]
pub(crate) enum LastStreamId {
    Incrementable(StreamId),
    GoAway(StreamId),
}

impl LastStreamId {
    pub(crate) const fn new() -> Self {
        Self::Incrementable(StreamId::ZERO)
    }

    /// check given id.
    /// return true when it's an idle id (higher than last stream id)
    pub(crate) fn check_idle(self, id: StreamId) -> bool {
        match self {
            Self::Incrementable(last_id) | Self::GoAway(last_id) => id > last_id,
        }
    }

    /// Advance the boundary to `id` if still incrementable. No-op once
    /// goaway, which is the mechanism by which post-GOAWAY HEADERS
    /// are dropped without bumping internal counters.
    pub(crate) fn try_set(&mut self, id: StreamId) -> Result<Option<()>, Error> {
        match self {
            Self::GoAway(_) => Ok(None),
            Self::Incrementable(last_id) if !id.is_client_initiated() || id <= *last_id => {
                Err(Error::GoAway(Reason::PROTOCOL_ERROR))
            }
            Self::Incrementable(last_id) => {
                *last_id = id;
                Ok(Some(()))
            }
        }
    }

    /// try transit into GoAway state. Returns Some(last_stream_id) when succeeded
    pub(crate) fn try_go_away(&mut self) -> Option<StreamId> {
        match *self {
            Self::Incrementable(id) => {
                let _ = mem::replace(self, LastStreamId::GoAway(id));
                Some(id)
            }
            _ => None,
        }
    }
}
