use crate::bytes::BufMut;

use super::{
    super::error::Error,
    head::{Head, Kind},
    stream_id::StreamId,
};

/// A decoded PING frame.
///
/// Carries the opaque 8-byte payload and whether this is an ACK (flag 0x1).
pub struct Ping {
    pub payload: [u8; 8],
    pub is_ack: bool,
}

impl Ping {
    pub const fn new(payload: [u8; 8], is_ack: bool) -> Self {
        Self { payload, is_ack }
    }

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
        let payload = payload.try_into().map_err(|_| Error::BadFrameSize)?;
        Ok(Ping { payload, is_ack })
    }

    pub fn encode<B>(&self, dst: &mut B)
    where
        B: BufMut,
    {
        let flag = if self.is_ack { 0x1 } else { 0x0 };
        Head::new(Kind::Ping, flag, StreamId::zero()).encode(8, dst);
        dst.put_slice(&self.payload);
    }
}
