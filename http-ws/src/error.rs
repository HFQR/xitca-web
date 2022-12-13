use std::{error, fmt};

use super::proto::OpCode;

/// WebSocket protocol errors.
#[derive(Debug)]
pub enum ProtocolError {
    UnmaskedFrame,
    MaskedFrame,
    InvalidOpcode(u8),
    InvalidLength(usize),
    BadOpCode,
    Overflow,
    ContinuationNotStarted,
    ContinuationStarted,
    ContinuationFragment(OpCode),
    Closed,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnmaskedFrame => f.write_str("Received an unmasked frame from client."),
            Self::MaskedFrame => f.write_str("Received a masked frame from server."),
            Self::InvalidOpcode(code) => write!(f, " Encountered invalid OpCode: {code}"),
            Self::InvalidLength(len) => write!(f, "Invalid control frame length: {len}."),
            Self::BadOpCode => f.write_str("Bad opcode."),
            Self::Overflow => f.write_str("A payload reached size limit."),
            Self::ContinuationNotStarted => f.write_str("Continuation is not started."),
            Self::ContinuationStarted => f.write_str("Received new continuation but it is already started."),
            Self::ContinuationFragment(ref code) => write!(f, "Unknown continuation fragment with OpCode: {code}."),
            Self::Closed => f.write_str("Connection already closed."),
        }
    }
}

impl error::Error for ProtocolError {}

impl From<OpCode> for ProtocolError {
    fn from(e: OpCode) -> Self {
        Self::ContinuationFragment(e)
    }
}

/// WebSocket handshake errors
#[derive(Debug, Eq, PartialEq)]
pub enum HandshakeError {
    GetMethodRequired,
    ConnectMethodRequired,
    NoWebsocketUpgrade,
    NoConnectionUpgrade,
    NoVersionHeader,
    UnsupportedVersion,
    BadWebsocketKey,
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::GetMethodRequired => f.write_str("Only GET method is allowed for HTTP/1.1 websocket."),
            Self::ConnectMethodRequired => f.write_str("Only CONNECT method is allowed HTTP/2 websocket."),
            Self::NoWebsocketUpgrade => f.write_str("Upgrade header is not set to HTTP/1.1 websocket."),
            Self::NoConnectionUpgrade => f.write_str("Connection header is not set to HTTP/1.1 websocket."),
            Self::NoVersionHeader => f.write_str(" WebSocket version header is not set to HTTP/1.1 websocket."),
            Self::UnsupportedVersion => f.write_str("Unsupported WebSocket version."),
            Self::BadWebsocketKey => f.write_str("WebSocket key is not set or wrong to HTTP/1.1 websocket."),
        }
    }
}

impl error::Error for HandshakeError {}
