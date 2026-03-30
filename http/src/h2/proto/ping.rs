use super::{error::Error, head::Head, reason::Reason};

/// A decoded PING frame.
///
/// Carries the opaque 8-byte payload and whether this is an ACK (flag 0x1).
pub struct Ping {
    pub payload: [u8; 8],
    pub is_ack: bool,
}

impl Ping {
    /// Load and validate a PING frame.
    ///
    /// Returns `Err(GoAway(PROTOCOL_ERROR))` if the stream ID is non-zero
    /// (RFC 7540 §6.7) and `Err(GoAway(FRAME_SIZE_ERROR))` if the payload
    /// is not exactly 8 bytes.
    pub fn load(head: Head, payload: &[u8]) -> Result<Ping, Error> {
        if !head.stream_id().is_zero() {
            return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
        }
        let is_ack = head.flag() & 0x1 == 0x1;
        let payload: [u8; 8] = payload
            .try_into()
            .map_err(|_| Error::GoAway(Reason::FRAME_SIZE_ERROR))?;
        Ok(Ping { payload, is_ack })
    }
}
