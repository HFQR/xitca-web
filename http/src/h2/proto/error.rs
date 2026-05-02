use super::{frame::reason::Reason, hpack::DecoderError};

/// Errors that can occur during parsing an HTTP/2 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A length value other than 8 was set on a PING message.
    BadFrameSize,

    /// The padding length was larger than the frame-header-specified
    /// length of the payload.
    TooMuchPadding,

    /// An invalid setting value was provided
    InvalidSettingValue,

    /// SETTINGS_INITIAL_WINDOW_SIZE caused the flow-control window to overflow.
    InitialWindowOverflow,

    /// The payload length specified by the frame header was not the
    /// value necessary for the specific frame type.
    InvalidPayloadLength,

    /// Like InvalidPayloadLength but stream-level
    InvalidPayloadLengthReset,

    /// Received a payload with an ACK settings frame
    InvalidPayloadAckSettings,

    /// An invalid stream identifier was provided.
    ///
    /// This is returned if a SETTINGS or PING frame is received with a stream
    /// identifier other than zero.
    InvalidStreamId,

    /// A request or response is malformed.
    MalformedMessage,

    /// An invalid stream dependency ID was provided
    ///
    /// This is returned if a HEADERS or PRIORITY frame is received with an
    /// invalid stream identifier.
    InvalidDependencyId,

    FrameAfterReset,

    FrameAfterEndStream,

    /// Failed to perform HPACK decoding
    Hpack(DecoderError),
}

impl From<DecoderError> for Error {
    fn from(e: DecoderError) -> Self {
        Self::Hpack(e)
    }
}

impl Error {
    pub(crate) fn reason(&self) -> Reason {
        match self {
            Self::BadFrameSize | Self::InvalidPayloadLength | Self::InvalidPayloadLengthReset| Self::InvalidPayloadAckSettings => {
                Reason::FRAME_SIZE_ERROR
            }
            Self::TooMuchPadding
            | Self::InvalidSettingValue
            // | Self::InvalidWindowUpdateValue
            | Self::InvalidStreamId
            | Self::MalformedMessage
            | Self::InvalidDependencyId
            | Self::FrameAfterEndStream => Reason::PROTOCOL_ERROR,
            Self::InitialWindowOverflow => Reason::FLOW_CONTROL_ERROR,
            Self::FrameAfterReset => Reason::STREAM_CLOSED,
            Self::Hpack(_) => Reason::COMPRESSION_ERROR,
        }
    }

    pub(crate) fn is_go_away(&self) -> bool {
        !matches!(
            self,
            Self::MalformedMessage | Self::InvalidDependencyId | Self::InvalidPayloadLengthReset
        )
    }
}
