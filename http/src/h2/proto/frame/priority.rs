use crate::bytes::BytesMut;

use super::{super::error::Error, head::Head, stream_id::StreamId};

#[derive(Debug, Eq, PartialEq)]
pub struct Priority {
    stream_id: StreamId,
    dependency: StreamDependency,
}

#[derive(Debug, Eq, PartialEq)]
pub struct StreamDependency {
    /// The ID of the stream dependency target
    dependency_id: StreamId,
    /// The weight for the stream. The value exposed (and set) here is always in
    /// the range [0, 255], instead of [1, 256] (as defined in section 5.3.2.)
    /// so that the value fits into a `u8`.
    weight: u8,
    /// True if the stream dependency is exclusive.
    is_exclusive: bool,
}

impl Priority {
    pub fn load(head: Head, src: &[u8]) -> Result<Self, Error> {
        let stream_id = head.stream_id();

        if stream_id == StreamId::ZERO {
            return Err(Error::InvalidStreamId);
        }

        let dependency = StreamDependency::load(src)?;

        if dependency.dependency_id() == stream_id {
            return Err(Error::InvalidDependencyId);
        }

        Ok(Priority { stream_id, dependency })
    }

    pub(super) fn _load(head: Head, src: &mut BytesMut) -> Result<Self, Error> {
        if src.len() < 5 {
            return Err(Error::InvalidPayloadLength);
        }

        let src = src.split_to(5);

        Self::load(head, &src)
    }
}

// ===== impl StreamDependency =====

impl StreamDependency {
    pub fn new(dependency_id: StreamId, weight: u8, is_exclusive: bool) -> Self {
        StreamDependency {
            dependency_id,
            weight,
            is_exclusive,
        }
    }

    pub fn load(src: &[u8]) -> Result<Self, Error> {
        if src.len() != 5 {
            return Err(Error::InvalidPayloadLengthReset);
        }

        // Parse the stream ID and exclusive flag
        let (dependency_id, is_exclusive) = StreamId::parse(&src[..4]);

        // Read the weight
        let weight = src[4];

        Ok(StreamDependency::new(dependency_id, weight, is_exclusive))
    }

    pub fn dependency_id(&self) -> StreamId {
        self.dependency_id
    }
}
