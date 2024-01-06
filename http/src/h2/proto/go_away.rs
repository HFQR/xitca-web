use core::fmt;

use tracing::trace;

use crate::bytes::{BufMut, Bytes};

use super::{
    error::Error,
    head::{Head, Kind},
    reason::Reason,
    stream_id::StreamId,
    unpack_octets_4,
};

#[derive(Clone, Eq, PartialEq)]
pub struct GoAway {
    last_stream_id: StreamId,
    error_code: Reason,
    debug_data: Bytes,
}

impl GoAway {
    pub fn new(last_stream_id: StreamId, reason: Reason) -> Self {
        GoAway {
            last_stream_id,
            error_code: reason,
            debug_data: Bytes::new(),
        }
    }

    pub fn with_debug_data(last_stream_id: StreamId, reason: Reason, debug_data: Bytes) -> Self {
        Self {
            last_stream_id,
            error_code: reason,
            debug_data,
        }
    }

    pub fn last_stream_id(&self) -> StreamId {
        self.last_stream_id
    }

    pub fn reason(&self) -> Reason {
        self.error_code
    }

    pub fn debug_data(&self) -> &Bytes {
        &self.debug_data
    }

    pub fn load(payload: &[u8]) -> Result<GoAway, Error> {
        if payload.len() < 8 {
            todo!()
            // return Err(Error::BadFrameSize);
        }

        let (last_stream_id, _) = StreamId::parse(&payload[..4]);
        let error_code = unpack_octets_4!(payload, 4, u32);
        let debug_data = Bytes::copy_from_slice(&payload[8..]);

        Ok(GoAway {
            last_stream_id,
            error_code: error_code.into(),
            debug_data,
        })
    }

    pub fn encode<B: BufMut>(&self, dst: &mut B) {
        trace!("encoding GO_AWAY; code={:?}", self.error_code);
        let head = Head::new(Kind::GoAway, 0, StreamId::zero());
        head.encode(8 + self.debug_data.len(), dst);
        dst.put_u32(self.last_stream_id.into());
        dst.put_u32(self.error_code.into());
        dst.put(self.debug_data.slice(..));
    }
}

impl fmt::Debug for GoAway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut builder = f.debug_struct("GoAway");
        builder.field("error_code", &self.error_code);
        builder.field("last_stream_id", &self.last_stream_id);

        if !self.debug_data.is_empty() {
            builder.field("debug_data", &self.debug_data);
        }

        builder.finish()
    }
}
