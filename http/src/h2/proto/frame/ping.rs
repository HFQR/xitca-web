use super::{super::error::Error, head::Head};

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
    /// Returns `Err(InvalidStreamId)` if the stream ID is non-zero
    /// (RFC 7540 §6.7) and `Err(BadFrameSize)` if the payload is not
    /// exactly 8 bytes.
    pub fn load(head: Head, payload: &[u8]) -> Result<Ping, Error> {
        if !head.stream_id().is_zero() {
            return Err(Error::InvalidStreamId);
        }
        let is_ack = head.flag() & 0x1 == 0x1;
        let payload: [u8; 8] = payload.try_into().map_err(|_| Error::BadFrameSize)?;
        Ok(Ping { payload, is_ack })
    }
}
