use std::io;

use super::proto::OpCode;

/// WebSocket protocol errors.
#[derive(Debug)]
pub enum ProtocolError {
    /// Received an unmasked frame from client.
    UnmaskedFrame,

    /// Received a masked frame from server.
    MaskedFrame,

    /// Encountered invalid opcode.
    InvalidOpcode(u8),

    /// Invalid control frame length
    InvalidLength(usize),

    /// Bad opcode.
    BadOpCode,

    /// A payload reached size limit.
    Overflow,

    /// Continuation is not started.
    ContinuationNotStarted,

    /// Received new continuation but it is already started.
    ContinuationStarted,

    /// Unknown continuation fragment.
    ContinuationFragment(OpCode),

    /// I/O error.
    Io(io::Error),
}

impl From<OpCode> for ProtocolError {
    fn from(e: OpCode) -> Self {
        Self::ContinuationFragment(e)
    }
}

impl From<io::Error> for ProtocolError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// WebSocket handshake errors
#[derive(PartialEq, Debug)]
pub enum HandshakeError {
    /// Only get method is allowed.
    GetMethodRequired,

    /// Upgrade header if not set to WebSocket.
    NoWebsocketUpgrade,

    /// Connection header is not set to upgrade.
    NoConnectionUpgrade,

    /// WebSocket version header is not set.
    NoVersionHeader,

    /// Unsupported WebSocket version.
    UnsupportedVersion,

    /// WebSocket key is not set or wrong.
    BadWebsocketKey,
}
